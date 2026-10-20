import { afterEach, describe, expect, it } from 'vitest';
import { nextTick } from 'vue';
import { allTabs, panes } from '../layout';
import { activeDrag, SESSION_MIME, TAB_MIME } from '../useDeckDrag';
import DropOverlay, { zoneAt } from './DropOverlay.vue';
import { buildDeck, dragEvent, mountWithDeck, pane, sess, sTab } from './paneTestKit';

const rect = { left: 0, top: 0, width: 400, height: 200 };

function setup() {
  const kit = buildDeck({
    sessions: [sess('a', 'p1'), sess('b', 'p1'), sess('c', 'p1'), sess('x', 'p2')],
    projectLayouts: { p1: pane('pa', [sTab('a'), sTab('b')]) },
  });
  const w = mountWithDeck(kit.deck, DropOverlay, { paneId: 'pa' });
  return { ...kit, w };
}

async function startDrag(w: ReturnType<typeof setup>['w'], tab = sTab('b')) {
  activeDrag.value = tab;
  await nextTick();
  const el = w.get('[data-testid="drop-overlay"]').element as HTMLElement;
  el.getBoundingClientRect = () => ({ ...rect, right: 400, bottom: 200, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
  return el;
}

afterEach(() => {
  activeDrag.value = null;
});

describe('zoneAt', () => {
  it('maps the 25% edge bands and the centre', () => {
    expect(zoneAt(10, 100, rect)).toBe('left');
    expect(zoneAt(390, 100, rect)).toBe('right');
    expect(zoneAt(200, 10, rect)).toBe('top');
    expect(zoneAt(200, 190, rect)).toBe('bottom');
    expect(zoneAt(200, 100, rect)).toBe('center');
    expect(zoneAt(100, 100, rect)).toBe('center');
  });

  it('picks the nearer edge in a corner', () => {
    expect(zoneAt(5, 40, rect)).toBe('left');
    expect(zoneAt(60, 3, rect)).toBe('top');
  });
});

describe('DropOverlay', () => {
  it('renders nothing unless a deck drag is active', async () => {
    const { w } = setup();
    expect(w.find('[data-testid="drop-overlay"]').exists()).toBe(false);
    await startDrag(w);
    expect(w.find('[data-testid="drop-overlay"]').exists()).toBe(true);
    w.unmount();
  });

  it('highlights the resulting half when the zone is allowed', async () => {
    const { w } = setup();
    const el = await startDrag(w);
    const { event } = dragEvent(el, 'dragover', { x: 10, y: 100 });
    await nextTick();
    const area = w.get('[data-testid="drop-area"]');
    expect(event.defaultPrevented).toBe(true);
    expect(area.attributes('data-zone')).toBe('left');
    expect(area.classes()).toContain('opacity-100');
    expect(area.classes()).toContain('left-0');
    expect(area.classes()).toContain('w-1/2');
    expect(area.classes()).toContain('bg-deck-accent/15');
    w.unmount();
  });

  it('shows nothing for a refused zone (duplicate centre) and does not accept the drop', async () => {
    const { w } = setup();
    const el = await startDrag(w);
    const { event } = dragEvent(el, 'dragover', { x: 200, y: 100 });
    await nextTick();
    expect(event.defaultPrevented).toBe(false);
    expect(w.get('[data-testid="drop-area"]').classes()).toContain('opacity-0');
    expect(w.find('[data-testid="drop-hint"]').exists()).toBe(false);
    w.unmount();
  });

  it('shows the chip for a session of another project in a project view', async () => {
    const { w } = setup();
    const el = await startDrag(w, sTab('x'));
    dragEvent(el, 'dragover', { x: 200, y: 100 });
    await nextTick();
    expect(w.get('[data-testid="drop-hint"]').text()).toBe('Add sessions from other projects to a group');
    expect(w.get('[data-testid="drop-area"]').classes()).toContain('opacity-0');
    w.unmount();
  });

  it('drop calls dropTab: an edge splits, and the overlay goes away', async () => {
    const { deck, w } = setup();
    const el = await startDrag(w);
    dragEvent(el, 'dragover', { x: 390, y: 100 });
    dragEvent(el, 'drop', { x: 390, y: 100, data: { [TAB_MIME]: JSON.stringify(sTab('b')) } });
    await nextTick();
    expect(panes(deck.currentLayout.value)).toHaveLength(2);
    expect(activeDrag.value).toBeNull();
    expect(w.find('[data-testid="drop-overlay"]').exists()).toBe(false);
    w.unmount();
  });

  it('accepts a sidebar session drag and turns it into a session tab', async () => {
    const { deck, w } = setup();
    const el = await startDrag(w, sTab('c'));
    dragEvent(el, 'drop', { x: 200, y: 100, data: { [SESSION_MIME]: JSON.stringify({ deck_id: 'c' }) } });
    await nextTick();
    expect(allTabs(deck.currentLayout.value)).toContainEqual(sTab('c'));
    w.unmount();
  });

  it('dragleave hides the highlight', async () => {
    const { w } = setup();
    const el = await startDrag(w);
    dragEvent(el, 'dragover', { x: 10, y: 100 });
    dragEvent(el, 'dragleave');
    await nextTick();
    expect(w.get('[data-testid="drop-area"]').classes()).toContain('opacity-0');
    w.unmount();
  });
});
