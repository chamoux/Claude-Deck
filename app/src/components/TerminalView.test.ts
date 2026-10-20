import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h } from 'vue';
import type { Target } from '../protocol';
import { fakeTransport } from '../transport';
import { createDeck, provideDeck } from '../useDeck';
import TerminalView from './TerminalView.vue';

interface FakeTerm {
  rows: number;
  cols: number;
  opened: HTMLElement | null;
  writes: Uint8Array[];
  calls: string[];
  disposed: boolean;
  dataCb: ((s: string) => void) | null;
  binaryCb: ((s: string) => void) | null;
  focus: () => void;
}
interface FakeObserver {
  disconnected: boolean;
}

const h$ = vi.hoisted(() => ({
  terms: [] as unknown[],
  observers: [] as unknown[],
}));
const terms = (): FakeTerm[] => h$.terms as FakeTerm[];
const observers = (): FakeObserver[] => h$.observers as FakeObserver[];

vi.mock('@xterm/xterm/css/xterm.css', () => ({}));
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    rows = 24;
    cols = 80;
    opened: HTMLElement | null = null;
    writes: Uint8Array[] = [];
    calls: string[] = [];
    disposed = false;
    dataCb: ((s: string) => void) | null = null;
    binaryCb: ((s: string) => void) | null = null;
    focus = vi.fn();
    constructor() {
      h$.terms.push(this);
    }
    loadAddon() {}
    open(el: HTMLElement) {
      this.opened = el;
    }
    write(d: Uint8Array) {
      this.calls.push('write');
      this.writes.push(d);
    }
    reset() {
      this.calls.push('reset');
    }
    dispose() {
      this.disposed = true;
    }
    onData(cb: (s: string) => void) {
      this.dataCb = cb;
      return { dispose() {} };
    }
    onBinary(cb: (s: string) => void) {
      this.binaryCb = cb;
      return { dispose() {} };
    }
  },
}));
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit() {}
  },
}));

class FakeRO {
  disconnected = false;
  constructor(public cb: () => void) {
    h$.observers.push(this);
  }
  observe() {}
  unobserve() {}
  disconnect() {
    this.disconnected = true;
  }
}

const a: Target = { kind: 'session', deck_id: 'a' };
const b: Target = { kind: 'shell', project_id: 'p1' };

function setup(target: Target) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  t.sent.length = 0;
  const Host = defineComponent({
    props: { target: { type: Object as () => Target, required: true } },
    setup(props) {
      provideDeck(deck);
      return () => h(TerminalView, { target: props.target });
    },
  });
  const wrapper = mount(Host, { props: { target }, attachTo: document.body });
  return { t, wrapper };
}

describe('TerminalView', () => {
  beforeEach(() => {
    h$.terms.length = 0;
    h$.observers.length = 0;
    vi.stubGlobal('ResizeObserver', FakeRO);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('opens the terminal and sends Attach with its rows and cols', () => {
    const { t } = setup(a);
    expect(terms()[0]?.opened).toBeInstanceOf(HTMLElement);
    expect(t.sent).toEqual([{ type: 'attach', target: a, rows: 24, cols: 80 }]);
  });

  it('Attached then output gives reset then write, in order', () => {
    const { t } = setup(a);
    const term = terms()[0]!;
    t.emit({ type: 'attached', target: a, index: 7 });
    const bytes = new Uint8Array([104, 105]);
    t.emitOutput(7, bytes);
    expect(term.calls).toEqual(['reset', 'write']);
    expect(term.writes[0]).toEqual(bytes);
  });

  it('ignores output for another index', () => {
    const { t } = setup(a);
    t.emit({ type: 'attached', target: a, index: 7 });
    t.emitOutput(8, new Uint8Array([1]));
    expect(terms()[0]!.writes).toHaveLength(0);
  });

  it('onData sends input for the right index', () => {
    const { t } = setup(a);
    t.emit({ type: 'attached', target: a, index: 7 });
    terms()[0]!.dataCb!('a');
    expect(t.inputs).toHaveLength(1);
    expect(t.inputs[0]?.index).toBe(7);
    expect(Array.from(t.inputs[0]!.data)).toEqual([97]);
  });

  it('onBinary sends latin1 bytes', () => {
    const { t } = setup(a);
    t.emit({ type: 'attached', target: a, index: 7 });
    terms()[0]!.binaryCb!(String.fromCharCode(255, 1));
    expect(Array.from(t.inputs[0]!.data)).toEqual([255, 1]);
  });

  it('changing target detaches the old one and attaches the new one', async () => {
    const { t, wrapper } = setup(a);
    t.sent.length = 0;
    await wrapper.setProps({ target: b });
    expect(t.sent).toEqual([
      { type: 'detach', target: a },
      { type: 'attach', target: b, rows: 24, cols: 80 },
    ]);
    expect(terms()[0]!.calls).toContain('reset');
  });

  it('unmount sends Detach and disposes', () => {
    const { t, wrapper } = setup(a);
    t.sent.length = 0;
    wrapper.unmount();
    expect(t.sent).toEqual([{ type: 'detach', target: a }]);
    expect(terms()[0]!.disposed).toBe(true);
    expect(observers()[0]!.disconnected).toBe(true);
  });

  it('re-attaches after a reconnect and applies the fresh snapshot', async () => {
    const { t } = setup(a);
    t.emit({ type: 'attached', target: a, index: 7 });
    t.sent.length = 0;
    t.emitConnection('reconnecting');
    await Promise.resolve();
    expect(t.sent).toEqual([]);
    t.emitConnection('connected');
    await Promise.resolve();
    expect(t.sent).toEqual([{ type: 'attach', target: a, rows: 24, cols: 80 }]);
    // Old index is gone; new index (reassigned by the helper) works.
    t.emitOutput(7, new Uint8Array([1]));
    expect(terms()[0]!.writes).toHaveLength(0);
    t.emit({ type: 'attached', target: a, index: 2 });
    t.emitOutput(2, new Uint8Array([2]));
    expect(terms()[0]!.writes).toHaveLength(1);
  });

  it('exposes focus()', () => {
    const { wrapper } = setup(a);
    const tv = wrapper.findComponent(TerminalView).vm as unknown as { focus(): void };
    tv.focus();
    expect(terms()[0]!.focus).toHaveBeenCalled();
  });
});
