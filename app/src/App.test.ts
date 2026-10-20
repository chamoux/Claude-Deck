import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LayoutNode } from './layout';
import type { Project, SessionState, SessionView, StateView } from './protocol';
import { fakeTransport } from './transport';
import { createDeck, deckKey } from './useDeck';
import App from './App.vue';

const h$ = vi.hoisted(() => ({ terms: [] as unknown[] }));

vi.mock('@xterm/xterm/css/xterm.css', () => ({}));
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    rows = 24;
    cols = 80;
    writes: Uint8Array[] = [];
    focus = vi.fn();
    constructor() {
      h$.terms.push(this);
    }
    loadAddon() {}
    open() {}
    write(d: Uint8Array) {
      this.writes.push(d);
    }
    reset() {}
    dispose() {}
    onData() {
      return { dispose() {} };
    }
    onBinary() {
      return { dispose() {} };
    }
  },
}));
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit() {}
  },
}));

const settings: StateView['settings'] = { auto_sleep_minutes: null, claude_path: null, window: null, shell: 'power_shell' };
const project = (id: string, layout?: LayoutNode): Project => ({ id, path: `C:/code/${id}`, name: id, shell_panel: { open: false, width: 480 }, layout });
const pane = (id: string, ...ids: string[]): LayoutNode => ({ type: 'pane', id, tabs: ids.map((deck_id) => ({ kind: 'session', deck_id })), active: 0 });
const sess = (deck_id: string, project_id: string, state: SessionState, name: string | null = null): SessionView => ({
  deck_id,
  project_id,
  name,
  state,
  background_work: false,
  error: null,
});

function build(state?: Partial<StateView>) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  t.emit({
    type: 'state',
    state: {
      projects: [project('p1'), project('p2')],
      sessions: [sess('a', 'p1', 'idle', 'Alpha'), sess('b', 'p2', 'idle', 'Beta')],
      shells: [],
      settings,
      missing_folders: [],
      ...state,
    },
  });
  const w = mount(App, { global: { provide: { [deckKey as symbol]: deck } }, attachTo: document.body });
  return { t, deck, w };
}

const last = <T,>(a: T[]): T | undefined => a[a.length - 1];

function key(init: KeyboardEventInit, target: EventTarget = document.body): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  return e;
}

/** A textarea inside an .xterm container, like xterm's hidden input. */
function xtermTextarea(): HTMLTextAreaElement {
  const xterm = document.createElement('div');
  xterm.className = 'xterm';
  const ta = document.createElement('textarea');
  xterm.append(ta);
  document.body.append(xterm);
  return ta;
}

describe('App', () => {
  beforeEach(() => {
    h$.terms.length = 0;
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    document.body.innerHTML = '';
  });

  it('empty state with no projects', () => {
    const { w } = build({ projects: [], sessions: [] });
    expect(w.text()).toContain('Add a project folder to start');
    expect(w.find('[data-testid="empty-add-project"]').exists()).toBe(true);
    w.unmount();
  });

  it('project with no open tabs offers New Claude session and its sessions', async () => {
    const { t, w } = build();
    expect(w.get('[data-testid="no-tabs"]').text()).toContain('Alpha');
    t.sent.length = 0;
    await w.get('[data-testid="empty-new-session"]').trigger('click');
    expect(t.sent).toEqual([{ type: 'new_session', project_id: 'p1' }]);
    w.unmount();
  });

  it('scripted flow: select, open, attach, output, needs_you', async () => {
    const { t, deck, w } = build();
    expect(deck.selectedProjectId.value).toBe('p1');
    deck.selectProject('p2');
    await w.get('[data-testid="session-b"] [data-nav]').trigger('click');
    await flushPromises();
    expect(t.sent).toContainEqual({ type: 'attach', target: { kind: 'session', deck_id: 'b' }, rows: 24, cols: 80 });

    t.emit({ type: 'attached', target: { kind: 'session', deck_id: 'b' }, index: 5 });
    t.emitOutput(5, new Uint8Array([104]));
    expect((h$.terms[0] as { writes: Uint8Array[] }).writes).toHaveLength(1);

    expect(last(t.attentions)).toEqual({ title: 'Claude Deck', flash: false });
    t.emit({ type: 'session_changed', session: sess('b', 'p2', 'needs_you', 'Beta') });
    await flushPromises();
    expect(last(t.attentions)).toEqual({ title: 'Claude Deck (1 needs you)', flash: true });
    // Sidebar row: amber tint plus the "Needs you" pill (design system).
    expect(w.get('[data-testid="session-b"] [data-nav]').classes()).toContain('bg-needs-soft');
    expect(w.find('[data-testid="session-b"] [data-testid="needs-pill"]').exists()).toBe(true);
    const tab = w.get('[data-testid="tab-b"]');
    expect(tab.classes()).toContain('bg-needs-soft');
    expect(tab.classes()).toContain('text-dot-needs');

    // Leaving Needs you updates the title and does not flash.
    t.emit({ type: 'session_changed', session: sess('b', 'p2', 'working', 'Beta') });
    await flushPromises();
    expect(last(t.attentions)).toEqual({ title: 'Claude Deck', flash: false });
    w.unmount();
  });

  it('Ctrl+` toggles the project terminal, even from inside a terminal', async () => {
    const { t, w } = build();
    await w.get('[data-testid="session-a"] [data-nav]').trigger('click');
    await flushPromises();
    t.sent.length = 0;
    key({ key: '`', code: 'Backquote', ctrlKey: true });
    expect(t.sent).toEqual([{ type: 'set_shell_panel', project_id: 'p1', panel: { open: true, width: 480 } }]);
    t.sent.length = 0;
    const e = key({ key: '`', code: 'Backquote', ctrlKey: true }, xtermTextarea());
    expect(e.defaultPrevented).toBe(true);
    expect(t.sent).toHaveLength(1);
    w.unmount();
  });

  it('shows the minimized rail for a project without an open panel', () => {
    const { w } = build();
    expect(w.find('[data-testid="shell-rail"]').exists()).toBe(true);
    w.unmount();
  });

  it('Ctrl+T, Ctrl+W, Ctrl+digit work outside a terminal and are left alone inside one', async () => {
    const { t, deck, w } = build();
    await w.get('[data-testid="session-a"] [data-nav]').trigger('click');
    await flushPromises();
    t.sent.length = 0;

    key({ key: 't', code: 'KeyT', ctrlKey: true });
    expect(t.sent).toEqual([{ type: 'new_session', project_id: 'p1' }]);

    const ta = xtermTextarea();
    t.sent.length = 0;
    for (const k of ['t', 'w', '2', 'c']) {
      expect(key({ key: k, ctrlKey: true }, ta).defaultPrevented).toBe(false);
    }
    expect(t.sent).toEqual([]);
    expect(deck.openTabs['p1']).toEqual(['a']);
    expect(deck.selectedProjectId.value).toBe('p1');

    key({ key: '2', ctrlKey: true });
    expect(deck.selectedProjectId.value).toBe('p2');
    key({ key: '1', ctrlKey: true });
    key({ key: 'w', ctrlKey: true });
    expect(deck.openTabs['p1']).toEqual([]);
    w.unmount();
  });

  it('Ctrl+Tab cycles the tabs of the focused pane, Changes included', () => {
    const { deck, w } = build({ sessions: [sess('a', 'p1', 'idle'), sess('c', 'p1', 'idle')] });
    deck.openSession('a');
    deck.openSession('c');
    deck.showChanges('p1');
    const active = () => (deck.currentLayout.value as { tabs: { kind: string; deck_id?: string }[]; active: number }).active;
    expect(active()).toBe(2);
    key({ key: 'Tab', ctrlKey: true });
    expect(active()).toBe(0);
    key({ key: 'Tab', ctrlKey: true, shiftKey: true });
    expect(active()).toBe(2);
    w.unmount();
  });

  it('Ctrl+backslash splits the focused pane to the right, even from inside a terminal', async () => {
    const { deck, w } = build({ sessions: [sess('a', 'p1', 'idle'), sess('c', 'p1', 'idle')] });
    deck.openSession('a');
    deck.openSession('c');
    await flushPromises();
    const e = key({ key: '\\', code: 'Backslash', ctrlKey: true }, xtermTextarea());
    expect(e.defaultPrevented).toBe(true);
    await flushPromises();
    expect(deck.currentLayout.value.type).toBe('split');
    expect(w.findAll('[data-pane-id]')).toHaveLength(2);
    w.unmount();
  });

  it('Ctrl+Alt+Arrow moves pane focus', async () => {
    const lay: LayoutNode = { type: 'split', dir: 'row', ratio: 0.5, a: pane('pa', 'a'), b: pane('pb', 'c') };
    const { deck, w } = build({ projects: [project('p1', lay), project('p2')], sessions: [sess('a', 'p1', 'idle'), sess('c', 'p1', 'idle')] });
    expect(deck.focusedPaneId.value).toBe('pa');
    key({ key: 'ArrowRight', ctrlKey: true, altKey: true });
    expect(deck.focusedPaneId.value).toBe('pb');
    key({ key: 'ArrowLeft', ctrlKey: true, altKey: true });
    expect(deck.focusedPaneId.value).toBe('pa');
    w.unmount();
  });

  it('selecting a group renders its 2x2 layout', async () => {
    const lay: LayoutNode = {
      type: 'split',
      dir: 'column',
      ratio: 0.5,
      a: { type: 'split', dir: 'row', ratio: 0.5, a: pane('g1', 'a'), b: pane('g2', 'b') },
      b: { type: 'split', dir: 'row', ratio: 0.5, a: pane('g3', 'c'), b: pane('g4', 'd') },
    };
    const { deck, w } = build({
      sessions: [sess('a', 'p1', 'idle'), sess('b', 'p2', 'idle'), sess('c', 'p1', 'idle'), sess('d', 'p2', 'asleep')],
      groups: [{ id: 'g', name: 'Billing', layout: lay }],
    });
    deck.selectGroup('g');
    await flushPromises();
    expect(w.findAll('[data-pane-id]')).toHaveLength(4);
    expect(w.find('[data-testid="asleep-panel"]').exists()).toBe(true);
    expect(w.find('[data-testid="no-tabs"]').exists()).toBe(false);
    w.unmount();
  });

  it('an empty group shows its drop hint and no project terminal', async () => {
    const { deck, w } = build({ groups: [{ id: 'g', name: 'Empty', layout: {} }] });
    deck.selectGroup('g');
    await flushPromises();
    expect(w.get('[data-testid="group-empty"]').text()).toContain('Empty group');
    expect(w.find('[data-testid="shell-rail"]').exists()).toBe(false);
    w.unmount();
  });

  it('the project terminal follows the focused pane project in a group', async () => {
    const lay: LayoutNode = { type: 'split', dir: 'row', ratio: 0.5, a: pane('g1', 'a'), b: pane('g2', 'b') };
    const { t, deck, w } = build({ groups: [{ id: 'g', name: 'Mix', layout: lay }] });
    deck.selectGroup('g');
    await flushPromises();
    expect(deck.focusedProjectId.value).toBe('p1');
    deck.focusPane('g2');
    await flushPromises();
    expect(deck.focusedProjectId.value).toBe('p2');
    t.sent.length = 0;
    key({ key: '`', code: 'Backquote', ctrlKey: true });
    expect(t.sent).toEqual([{ type: 'set_shell_panel', project_id: 'p2', panel: { open: true, width: 480 } }]);
    w.unmount();
  });

  it('shortcuts are ignored while a dialog is open', () => {
    const { t, deck, w } = build();
    deck.confirm.value = { message: 'sure?', yes: () => {} };
    t.sent.length = 0;
    key({ key: 't', ctrlKey: true });
    expect(t.sent).toEqual([]);
    w.unmount();
  });

  it('re-attaches the mounted terminal after a reconnect', async () => {
    const { t, w } = build();
    await w.get('[data-testid="session-a"] [data-nav]').trigger('click');
    await flushPromises();
    t.sent.length = 0;
    t.emitConnection('reconnecting');
    t.emitConnection('connected');
    await flushPromises();
    expect(t.sent).toEqual([{ type: 'attach', target: { kind: 'session', deck_id: 'a' }, rows: 24, cols: 80 }]);
    w.unmount();
  });
});
