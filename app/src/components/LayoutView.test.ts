import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import type { LayoutNode } from '../layout';
import { activeDrag } from '../useDeckDrag';
import LayoutView from './LayoutView.vue';
import { buildDeck, dragEvent, mountWithDeck, pane, sess, sTab, split } from './paneTestKit';

vi.mock('./TerminalView.vue', () => ({
  default: { name: 'TerminalView', props: ['target'], template: '<div data-testid="term">{{ target.deck_id }}</div>' },
}));

type RoCb = (entries: { contentRect: { width: number; height: number } }[]) => void;
let roCallbacks: { cb: RoCb; el: Element }[] = [];

beforeEach(() => {
  roCallbacks = [];
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(private cb: RoCb) {}
      observe(el: Element) {
        roCallbacks.push({ cb: this.cb, el });
      }
      disconnect() {}
      unobserve() {}
    },
  );
});
afterEach(() => {
  vi.unstubAllGlobals();
  activeDrag.value = null;
});

const grid: LayoutNode = split(
  'row',
  split('column', pane('p1', [sTab('a')]), pane('p2', [sTab('b')])),
  split('column', pane('p3', [sTab('c')]), pane('p4', [sTab('d')])),
);

function setup(layout: LayoutNode) {
  const kit = buildDeck({
    sessions: ['a', 'b', 'c', 'd'].map((id) => sess(id, 'p1')),
    projectLayouts: { p1: layout },
  });
  const w = mountWithDeck(kit.deck, LayoutView, () => ({ node: kit.deck.currentLayout.value, path: [] }));
  return { ...kit, w };
}

function ptr(el: Element, type: string, init: { clientX?: number; clientY?: number }): void {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, ...init });
  Object.defineProperty(e, 'pointerId', { value: 1 });
  el.dispatchEvent(e);
}

const rootRatio = (deck: ReturnType<typeof setup>['deck']): number => {
  const n = deck.currentLayout.value;
  return n.type === 'split' ? n.ratio : -1;
};

describe('LayoutView', () => {
  it('renders a 2x2 recursively: 4 panes, 3 splitters, row outside and columns inside', () => {
    const { w } = setup(grid);
    expect(w.findAll('section[data-pane-id]').map((p) => p.attributes('data-pane-id'))).toEqual(['p1', 'p2', 'p3', 'p4']);
    expect(w.findAll('[role="separator"]')).toHaveLength(3);
    expect(w.findAll('[data-testid="layout-split"]').map((s) => s.attributes('data-dir'))).toEqual(['row', 'column', 'column']);
    w.unmount();
  });

  it('sizes the two sides from the ratio', () => {
    const { w } = setup(split('row', pane('p1', [sTab('a')]), pane('p2', [sTab('b')]), 0.7));
    const sides = w.get('[data-testid="layout-split"]').element.children;
    expect((sides[0] as HTMLElement).style.flex).toContain('0.7');
    expect((sides[2] as HTMLElement).style.flex).toContain('0.3');
    w.unmount();
  });

  it('a horizontal splitter drag converts pixels to a ratio via setRatio', async () => {
    const { deck, t, w } = setup(split('row', pane('p1', [sTab('a')]), pane('p2', [sTab('b')])));
    await nextTick();
    roCallbacks[0]?.cb([{ contentRect: { width: 1000, height: 500 } }]);
    const sep = w.get('[role="separator"]').element;
    ptr(sep, 'pointerdown', { clientX: 500 });
    ptr(sep, 'pointermove', { clientX: 600 });
    expect(rootRatio(deck)).toBeCloseTo(0.6);
    ptr(sep, 'pointermove', { clientX: 650 });
    expect(rootRatio(deck)).toBeCloseTo(0.65);
    ptr(sep, 'pointerup', { clientX: 650 });
    await nextTick(); // the parent re-renders with the new node before the next gesture
    ptr(sep, 'pointerdown', { clientX: 650 });
    ptr(sep, 'pointermove', { clientX: 550 });
    expect(rootRatio(deck)).toBeCloseTo(0.55);
    ptr(sep, 'pointerup', { clientX: 550 });
    expect(t.sent).toEqual([]); // the save is debounced
    w.unmount();
  });

  it('clamps the ratio, and a column split drags vertically at its nested path', async () => {
    const { deck, w } = setup(grid);
    await nextTick();
    const colEl = w.findAll('[data-testid="layout-split"]')[1]!.element;
    roCallbacks.find((r) => r.el === colEl)?.cb([{ contentRect: { width: 400, height: 400 } }]);
    const sep = w.findAll('[role="separator"]')[0]!;
    expect(sep.attributes('aria-orientation')).toBe('horizontal');
    ptr(sep.element, 'pointerdown', { clientY: 200 });
    ptr(sep.element, 'pointermove', { clientY: 260 });
    const root = deck.currentLayout.value;
    if (root.type !== 'split' || root.a.type !== 'split') throw new Error('shape');
    expect(root.a.ratio).toBeCloseTo(0.65);
    expect(root.ratio).toBe(0.5);
    ptr(sep.element, 'pointermove', { clientY: 5000 });
    const r2 = deck.currentLayout.value;
    if (r2.type !== 'split' || r2.a.type !== 'split') throw new Error('shape');
    expect(r2.a.ratio).toBe(0.85);
    w.unmount();
  });

  it('double click resets to 0.5', () => {
    const { deck, w } = setup(split('row', pane('p1', [sTab('a')]), pane('p2', [sTab('b')]), 0.7));
    w.get('[role="separator"]').element.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    expect(rootRatio(deck)).toBe(0.5);
    w.unmount();
  });

  it('a tab drag anywhere shows the overlay on every pane (window-wide tracking)', async () => {
    const { w } = setup(grid);
    expect(w.findAll('[data-testid="drop-overlay"]')).toHaveLength(0);
    dragEvent(w.get('[data-testid="tab-a"]').element, 'dragstart');
    await nextTick();
    expect(w.findAll('[data-testid="drop-overlay"]')).toHaveLength(4);
    dragEvent(w.get('[data-testid="tab-a"]').element, 'dragend');
    await nextTick();
    expect(w.findAll('[data-testid="drop-overlay"]')).toHaveLength(0);
    w.unmount();
  });
});
