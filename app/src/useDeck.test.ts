import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { allTabs, panes } from './layout';
import type { LayoutNode, TabRef } from './layout';
import type { Group, Project, Request, SessionState, SessionView, StateView, Target } from './protocol';
import { fakeTransport } from './transport';
import { createDeck } from './useDeck';

const settings: StateView['settings'] = { auto_sleep_minutes: null, claude_path: null, window: null, shell: 'power_shell' };

function project(id: string): Project {
  return { id, path: `C:/code/${id}`, name: id, shell_panel: { open: false, width: 480 } };
}

function sess(deck_id: string, project_id: string, state: SessionState): SessionView {
  return { deck_id, project_id, name: null, state, background_work: false, error: null };
}

function stateOf(sessions: SessionView[]): StateView {
  return { projects: [project('p1'), project('p2')], sessions, shells: [], settings, missing_folders: [] };
}

function setup(sessions: SessionView[] = []) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  t.emit({ type: 'state', state: stateOf(sessions) });
  t.sent.length = 0;
  return { t, deck };
}

const sessionTarget = (deck_id: string): Target => ({ kind: 'session', deck_id });

describe('useDeck', () => {
  it('start sends Hello window', () => {
    const t = fakeTransport();
    createDeck(t).start();
    expect(t.sent).toEqual([{ type: 'hello', client: 'window' }]);
  });

  it('State event fills projects', () => {
    const { deck } = setup();
    expect(deck.projects.value.map((p) => p.id)).toEqual(['p1', 'p2']);
  });

  it('SessionChanged patches one session and inserts new ones', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle'), sess('b', 'p1', 'idle')]);
    t.emit({ type: 'session_changed', session: sess('a', 'p1', 'working') });
    expect(deck.session('a')?.state).toBe('working');
    expect(deck.session('b')?.state).toBe('idle');
    t.emit({ type: 'session_changed', session: sess('c', 'p2', 'idle') });
    expect(deck.sessionsOf('p2')).toHaveLength(1);
  });

  it('counts needs-you and drives the window title', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle'), sess('b', 'p2', 'idle')]);
    expect(deck.windowTitle.value).toBe('Claude Deck');
    t.emit({ type: 'session_changed', session: sess('a', 'p1', 'needs_you') });
    expect(deck.needsYouCount.value).toBe(1);
    expect(deck.windowTitle.value).toBe('Claude Deck (1 needs you)');
    t.emit({ type: 'session_changed', session: sess('b', 'p2', 'needs_you') });
    expect(deck.needsYouByProject.value).toEqual({ p1: 1, p2: 1 });
    expect(deck.windowTitle.value).toBe('Claude Deck (2 need you)');
  });

  it('openSession wakes an asleep session and opens the tab', () => {
    const { t, deck } = setup([sess('a', 'p1', 'asleep')]);
    deck.openSession('a');
    expect(t.sent).toEqual([{ type: 'wake', deck_id: 'a' }]);
    expect(deck.openTabs['p1']).toEqual(['a']);
    expect(deck.activeTab['p1']).toBe('a');
  });

  it('openSession on an awake session sends nothing', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle')]);
    deck.openSession('a');
    expect(t.sent).toEqual([]);
    expect(deck.openTabs['p1']).toEqual(['a']);
  });

  it('sleep sends force:false and never confirms by itself', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle'), sess('b', 'p1', 'working')]);
    deck.sleep('a');
    deck.sleep('b');
    expect(t.sent).toEqual([
      { type: 'sleep', deck_id: 'a', force: false },
      { type: 'sleep', deck_id: 'b', force: false },
    ]);
    expect(deck.confirm.value).toBeNull();
  });

  it('needs_confirm for sleep: yes resends with force', () => {
    const { t, deck } = setup([sess('b', 'p1', 'working')]);
    t.emit({ type: 'needs_confirm', action: { type: 'sleep', deck_id: 'b' }, message: 'Claude may be working. Stop it anyway?' });
    expect(deck.confirm.value?.message).toBe('Claude may be working. Stop it anyway?');
    expect(t.sent).toEqual([]);
    deck.confirmYes();
    expect(t.sent).toEqual([{ type: 'sleep', deck_id: 'b', force: true }]);
    expect(deck.confirm.value).toBeNull();
  });

  it('needs_confirm for close_session: yes resends with force', () => {
    const { t, deck } = setup([sess('b', 'p1', 'needs_you')]);
    t.emit({ type: 'needs_confirm', action: { type: 'close_session', deck_id: 'b' }, message: 'Close it?' });
    deck.confirmYes();
    expect(t.sent).toEqual([{ type: 'close_session', deck_id: 'b', force: true }]);
  });

  it('needs_confirm for remove_project: yes resends with force', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle')]);
    t.emit({ type: 'needs_confirm', action: { type: 'remove_project', project_id: 'p1' }, message: 'Remove?' });
    deck.confirmYes();
    expect(t.sent).toEqual([{ type: 'remove_project', project_id: 'p1', force: true }]);
  });

  it('confirmNo discards the pending action', () => {
    const { t, deck } = setup([sess('b', 'p1', 'needs_you')]);
    t.emit({ type: 'needs_confirm', action: { type: 'close_session', deck_id: 'b' }, message: 'x' });
    deck.confirmNo();
    expect(deck.confirm.value).toBeNull();
    expect(t.sent).toEqual([]);
  });

  it('closeSession removes the tab', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle')]);
    deck.openSession('a');
    deck.closeSession('a');
    expect(t.sent).toEqual([{ type: 'close_session', deck_id: 'a', force: false }]);
    expect(deck.openTabs['p1']).toEqual([]);
  });

  it('routes output to the sink of the attached index only', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle')]);
    const calls: string[] = [];
    deck.subscribeTerminal(sessionTarget('a'), {
      reset: () => calls.push('reset'),
      write: (d) => calls.push(`write:${d[0]}`),
    });
    t.emitOutput(7, Uint8Array.of(1)); // before Attached: dropped
    t.emit({ type: 'attached', target: sessionTarget('a'), index: 7 });
    t.emitOutput(7, Uint8Array.of(2));
    t.emitOutput(8, Uint8Array.of(3));
    expect(calls).toEqual(['reset', 'write:2']);
    t.emit({ type: 'detached', target: sessionTarget('a') });
    t.emitOutput(7, Uint8Array.of(4));
    expect(calls).toEqual(['reset', 'write:2']);
  });

  it('unsubscribing a sink stops delivery', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle')]);
    const calls: string[] = [];
    const off = deck.subscribeTerminal(sessionTarget('a'), { reset: () => calls.push('r'), write: () => calls.push('w') });
    t.emit({ type: 'attached', target: sessionTarget('a'), index: 1 });
    off();
    t.emitOutput(1, Uint8Array.of(1));
    expect(calls).toEqual(['r']);
  });

  it('input is a no-op until Attached', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle')]);
    deck.input(sessionTarget('a'), Uint8Array.of(65));
    expect(t.inputs).toEqual([]);
    t.emit({ type: 'attached', target: sessionTarget('a'), index: 3 });
    deck.input(sessionTarget('a'), Uint8Array.of(65));
    expect(t.inputs).toEqual([{ index: 3, data: Uint8Array.of(65) }]);
  });

  it('requestClose closes directly when nothing is busy', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle'), sess('b', 'p1', 'asleep')]);
    deck.requestClose();
    expect(t.closed).toEqual([false]);
    expect(deck.closeDialogOpen.value).toBe(false);
  });

  it('requestClose opens the dialog when a session is busy', () => {
    const { t, deck } = setup([sess('a', 'p1', 'working')]);
    deck.requestClose();
    expect(t.closed).toEqual([]);
    expect(deck.closeDialogOpen.value).toBe(true);
  });

  it('a close request from the window triggers requestClose', () => {
    const { t } = setup([sess('a', 'p1', 'idle')]);
    t.emitCloseRequested();
    expect(t.closed).toEqual([false]);
  });

  it('newSession opens the tab of the next unknown session of that project', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle')]);
    deck.newSession('p1');
    expect(t.sent).toEqual([{ type: 'new_session', project_id: 'p1' }]);
    t.emit({ type: 'session_changed', session: sess('a', 'p1', 'working') });
    expect(deck.openTabs['p1']).toEqual([]);
    t.emit({ type: 'session_changed', session: sess('n', 'p1', 'idle') });
    expect(deck.openTabs['p1']).toEqual(['n']);
    expect(deck.activeTab['p1']).toBe('n');
    t.emit({ type: 'session_changed', session: sess('m', 'p1', 'idle') });
    expect(deck.openTabs['p1']).toEqual(['n']);
  });

  it('Error events are shown', () => {
    const { t, deck } = setup();
    t.emit({ type: 'error', message: 'boom' });
    expect(deck.lastError.value).toBe('boom');
  });

  it('stores memory, git status and past sessions', () => {
    const { t, deck } = setup();
    t.emit({ type: 'memory', awake: 2, bytes: 5 });
    expect(deck.memory.value).toEqual({ awake: 2, bytes: 5 });
    t.emit({ type: 'git_status', project_id: 'p1', result: { status: 'not_git' } });
    expect(deck.changes['p1']).toEqual({ status: 'not_git' });
    t.emit({ type: 'past_sessions', project_id: 'p1', sessions: [{ id: 'x', last_prompt: null, modified_ms: 1 }] });
    expect(deck.pastSessions['p1']).toHaveLength(1);
  });

  it('refreshes changes when a session goes working to idle with the Changes tab active', () => {
    const { t, deck } = setup([sess('a', 'p1', 'working')]);
    deck.selectProject('p1');
    deck.showChanges('p1');
    expect(t.sent).toEqual([{ type: 'git_status', project_id: 'p1' }]);
    t.sent.length = 0;
    t.emit({ type: 'session_changed', session: sess('a', 'p1', 'idle') });
    expect(t.sent).toEqual([{ type: 'git_status', project_id: 'p1' }]);
  });

  it('removeProject sends force:false', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle')]);
    deck.removeProject('p1');
    expect(t.sent).toEqual([{ type: 'remove_project', project_id: 'p1', force: false }]);
    expect(deck.confirm.value).toBeNull();
  });

  it('reconnect clears terminal indices and bumps reattachGeneration', () => {
    const { t, deck } = setup([sess('a', 'p1', 'idle')]);
    const writes: string[] = [];
    deck.subscribeTerminal(sessionTarget('a'), { reset: () => {}, write: (d) => writes.push(new TextDecoder().decode(d)) });
    t.emit({ type: 'attached', target: sessionTarget('a'), index: 3 });
    t.emitOutput(3, new TextEncoder().encode('x'));
    expect(writes).toEqual(['x']);
    const gen = deck.reattachGeneration.value;
    t.emitConnection('reconnecting');
    expect(deck.reattachGeneration.value).toBe(gen);
    t.emitConnection('connected');
    expect(deck.reattachGeneration.value).toBe(gen + 1);
    t.emitOutput(3, new TextEncoder().encode('stale'));
    deck.input(sessionTarget('a'), new Uint8Array([1]));
    expect(writes).toEqual(['x']);
    expect(t.inputs).toEqual([]);
  });
});

// ---- Split panes and groups (Plan 4 T3) -------------------------------------------------------

const s_ = (deck_id: string): TabRef => ({ kind: 'session', deck_id });
const pane = (id: string, tabs: TabRef[], active = 0): LayoutNode => ({ type: 'pane', id, tabs, active });
const split = (a: LayoutNode, b: LayoutNode): LayoutNode => ({ type: 'split', dir: 'row', ratio: 0.5, a, b });

function setupGroups(
  sessions: SessionView[],
  groups: Group[] = [{ id: 'g1', name: 'Billing', layout: {} }],
  projects: Project[] = [project('p1'), project('p2')],
) {
  const t = fakeTransport();
  const deck = createDeck(t);
  deck.start();
  const st = (g: Group[], ps: Project[] = projects): StateView => ({ ...stateOf(sessions), projects: ps, groups: g });
  t.emit({ type: 'state', state: st(groups) });
  t.sent.length = 0;
  return { t, deck, st, projects };
}

describe('useDeck views, layouts and groups', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('selecting a group sets selected, hides selectedProjectId and shows its empty layout', () => {
    const { deck } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.selectProject('p1');
    expect(deck.selectedProjectId.value).toBe('p1');
    deck.selectGroup('g1');
    expect(deck.selected.value).toEqual({ kind: 'group', id: 'g1' });
    expect(deck.selectedProjectId.value).toBeNull();
    expect(deck.currentLayout.value).toMatchObject({ type: 'pane', tabs: [] });
    expect(deck.focusedPaneId.value).toBe((deck.currentLayout.value as { id: string }).id);
  });

  it('a pure load never persists (absent layout, {} group)', () => {
    const { t } = setupGroups([sess('a', 'p1', 'idle')]);
    vi.advanceTimersByTime(2000);
    expect(t.sent).toEqual([]);
  });

  it('dropTab in a group persists SetGroupLayout after 300 ms, debounced', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'idle'), sess('b', 'p2', 'idle')]);
    deck.selectGroup('g1');
    deck.openSession('a');
    const id = deck.focusedPaneId.value!;
    deck.dropTab(id, 'right', s_('b'));
    vi.advanceTimersByTime(299);
    expect(t.sent).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(t.sent).toHaveLength(1);
    const req = t.sent[0] as Extract<Request, { type: 'set_group_layout' }>;
    expect(req.type).toBe('set_group_layout');
    expect(req.group_id).toBe('g1');
    expect(req.layout).toMatchObject({ type: 'split', dir: 'row' });
    expect(deck.focusedProjectId.value).toBe('p2');
  });

  it('project edits send SetProjectLayout for that project only', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.selectProject('p1');
    deck.openSession('a');
    vi.advanceTimersByTime(300);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]).toMatchObject({ type: 'set_project_layout', project_id: 'p1' });
  });

  it('refuses sessions of another project in a project view, allows them in a group', () => {
    const { deck } = setupGroups([sess('a', 'p1', 'idle'), sess('b', 'p2', 'idle')]);
    deck.selectProject('p1');
    deck.openSession('a');
    const id = deck.focusedPaneId.value!;
    expect(deck.canDropHere(id, 'right', s_('b'))).toEqual({ ok: false, reason: 'other-project' });
    deck.dropTab(id, 'right', s_('b'));
    expect(deck.openTabs['p1']).toEqual(['a']);
    deck.selectGroup('g1');
    deck.openSession('a');
    const gid = deck.focusedPaneId.value!;
    expect(deck.canDropHere(gid, 'right', s_('b'))).toEqual({ ok: true });
  });

  it('canDropHere reports cap, duplicate and noop', () => {
    const ss = ['a', 'b', 'c', 'd', 'e'].map((x) => sess(x, 'p1', 'idle'));
    const { deck } = setupGroups(ss);
    deck.selectProject('p1');
    ss.forEach((s) => deck.openSession(s.deck_id));
    const first = deck.focusedPaneId.value!;
    expect(deck.canDropHere(first, 'center', s_('a'))).toEqual({ ok: false, reason: 'duplicate' });
    expect(deck.canDropHere('nope', 'left', s_('a'))).toEqual({ ok: false, reason: 'noop' });
    deck.dropTab(first, 'right', s_('b'));
    deck.dropTab(first, 'bottom', s_('c'));
    deck.dropTab(deck.focusedPaneId.value!, 'bottom', s_('d'));
    expect(panes(deck.currentLayout.value)).toHaveLength(4);
    expect(deck.canDropHere(first, 'left', s_('e'))).toEqual({ ok: false, reason: 'cap' });
  });

  it('prunes layouts when a session disappears from State, and persists only the changed view', () => {
    const groupLayout: Group = { id: 'g1', name: 'G', layout: pane('pg', [s_('a')]) };
    const { t, deck, st, projects } = setupGroups(
      [sess('a', 'p1', 'idle'), sess('b', 'p1', 'idle')],
      [groupLayout],
      [{ ...project('p1'), layout: pane('pa', [s_('a'), s_('b')]) }, project('p2')],
    );
    expect(deck.openTabs['p1']).toEqual(['a', 'b']);
    expect(t.sent).toEqual([]);
    t.emit({ type: 'state', state: { ...st([groupLayout], projects), sessions: [sess('a', 'p1', 'idle')] } });
    expect(deck.openTabs['p1']).toEqual(['a']);
    vi.advanceTimersByTime(300);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]).toMatchObject({ type: 'set_project_layout', project_id: 'p1' });
  });

  it('prunes a Changes tab when its project is removed', () => {
    const g: Group = { id: 'g1', name: 'G', layout: pane('pg', [{ kind: 'changes', project_id: 'p2' }]) };
    const { t, deck, st } = setupGroups([sess('a', 'p1', 'idle')], [g]);
    expect(allTabs(deck.layoutOf({ kind: 'group', id: 'g1' }))).toHaveLength(1);
    t.emit({ type: 'state', state: st([g], [project('p1')]) });
    expect(allTabs(deck.layoutOf({ kind: 'group', id: 'g1' }))).toHaveLength(0);
    vi.advanceTimersByTime(300);
    expect(t.sent).toEqual([expect.objectContaining({ type: 'set_group_layout', group_id: 'g1' })]);
  });

  it('prunes on SessionChanged for a session a project view no longer owns', () => {
    const { t, deck } = setupGroups(
      [sess('a', 'p1', 'idle')],
      [],
      [{ ...project('p1'), layout: pane('pa', [s_('a')]) }, project('p2')],
    );
    t.emit({ type: 'session_changed', session: sess('a', 'p2', 'idle') });
    expect(deck.openTabs['p1']).toEqual([]);
    vi.advanceTimersByTime(300);
    expect(t.sent).toHaveLength(1);
  });

  it('focusedProjectId follows the focused pane in a group', () => {
    const { deck } = setupGroups(
      [sess('a', 'p1', 'idle'), sess('b', 'p2', 'idle')],
      [{ id: 'g1', name: 'G', layout: split(pane('L', [s_('a')]), pane('R', [s_('b')])) }],
    );
    expect(deck.focusedProjectId.value).toBeNull();
    deck.selectGroup('g1');
    expect(deck.focusedProjectId.value).toBe('p1');
    deck.focusPane('R');
    expect(deck.focusedProjectId.value).toBe('p2');
    deck.moveFocus('left');
    expect(deck.focusedPaneId.value).toBe('L');
    deck.selectProject('p2');
    expect(deck.focusedProjectId.value).toBe('p2');
  });

  it('addToGroup adds to the group pane once and persists', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.addToGroup('g1', 'a');
    deck.addToGroup('g1', 'a');
    expect(allTabs(deck.layoutOf({ kind: 'group', id: 'g1' }))).toEqual([s_('a')]);
    vi.advanceTimersByTime(300);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]).toMatchObject({ type: 'set_group_layout', group_id: 'g1' });
  });

  it('adding two sessions to a group tiles them side by side and persists after the debounce', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'idle'), sess('b', 'p2', 'idle')]);
    deck.addToGroup('g1', 'a');
    deck.addToGroup('g1', 'b');
    const layout = deck.layoutOf({ kind: 'group', id: 'g1' });
    expect(layout).toMatchObject({ type: 'split', dir: 'row' });
    expect(panes(layout).map((p) => p.tabs)).toEqual([[s_('a')], [s_('b')]]);
    expect(t.sent).toEqual([]);
    vi.advanceTimersByTime(300);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]).toMatchObject({ type: 'set_group_layout', group_id: 'g1', layout });
  });

  it('tileGroup splits a one-pane two-tab group into two panes and persists', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'idle'), sess('b', 'p1', 'idle')], [
      { id: 'g1', name: 'Billing', layout: pane('x', [s_('a'), s_('b')]) },
    ]);
    deck.tileGroup('g1');
    const layout = deck.layoutOf({ kind: 'group', id: 'g1' });
    expect(panes(layout).map((p) => p.tabs)).toEqual([[s_('a')], [s_('b')]]);
    vi.advanceTimersByTime(300);
    expect(t.sent).toHaveLength(1);
    expect(t.sent[0]).toMatchObject({ type: 'set_group_layout', group_id: 'g1' });
    t.sent.length = 0;
    deck.tileGroup('nope'); // unknown group: ignored
    vi.advanceTimersByTime(300);
    expect(t.sent).toEqual([]);
  });

  it('newGroup sends AddGroup, then selects the group that arrives', () => {
    const { t, deck, st } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.newGroup('Billing', { addSession: 'a' });
    expect(t.sent).toEqual([{ type: 'add_group', name: 'Billing' }]);
    t.emit({
      type: 'state',
      state: st([
        { id: 'g1', name: 'Billing', layout: {} },
        { id: 'g2', name: 'Billing', layout: {} },
      ]),
    });
    expect(deck.selected.value).toEqual({ kind: 'group', id: 'g2' });
    expect(allTabs(deck.layoutOf({ kind: 'group', id: 'g2' }))).toEqual([s_('a')]);
  });

  it('renameGroup sends RenameGroup', () => {
    const { t, deck } = setupGroups([]);
    deck.renameGroup('g1', 'x');
    expect(t.sent).toEqual([{ type: 'rename_group', group_id: 'g1', name: 'x' }]);
  });

  it('removeGroup asks first; yes sends RemoveGroup; a vanished selected group falls back to a project', () => {
    const { t, deck, st } = setupGroups([]);
    deck.selectGroup('g1');
    deck.removeGroup('g1');
    expect(t.sent).toEqual([]);
    expect(deck.confirm.value?.message).toContain('Billing');
    deck.confirmYes();
    expect(t.sent).toEqual([{ type: 'remove_group', group_id: 'g1' }]);
    t.emit({ type: 'state', state: st([]) });
    expect(deck.selected.value).toEqual({ kind: 'project', id: 'p1' });
  });

  it('a server echo does not clobber a pending or just-saved local edit', () => {
    const { t, deck, st } = setupGroups([sess('a', 'p1', 'idle'), sess('b', 'p1', 'idle')]);
    const old = (): StateView => st([{ id: 'g1', name: 'Billing', layout: {} }]);
    deck.selectGroup('g1');
    deck.openSession('a');
    t.emit({ type: 'state', state: old() }); // broadcast while the edit is pending
    expect(allTabs(deck.currentLayout.value)).toEqual([s_('a')]);
    deck.openSession('b');
    vi.advanceTimersByTime(300);
    const sent = t.sent[0] as Extract<Request, { type: 'set_group_layout' }>;
    t.emit({ type: 'state', state: old() }); // stale broadcast right after our save
    expect(allTabs(deck.currentLayout.value)).toEqual([s_('a'), s_('b')]);
    t.emit({ type: 'state', state: st([{ id: 'g1', name: 'Billing', layout: sent.layout }]) }); // our echo
    vi.advanceTimersByTime(1000);
    expect(t.sent).toHaveLength(1);
    expect(allTabs(deck.currentLayout.value)).toEqual([s_('a'), s_('b')]);
  });

  it('adopts a changed server layout when nothing is pending', () => {
    const { t, deck, st } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.selectGroup('g1');
    t.emit({ type: 'state', state: st([{ id: 'g1', name: 'Billing', layout: pane('x', [s_('a')]) }]) });
    expect(allTabs(deck.currentLayout.value)).toEqual([s_('a')]);
    expect(t.sent).toEqual([]);
  });

  it('on reconnect, drops pending edits and re-adopts the server layout', () => {
    const { t, deck, st } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.selectGroup('g1');
    deck.openSession('a');
    t.emitConnection('reconnecting');
    t.emitConnection('connected');
    vi.advanceTimersByTime(1000);
    expect(t.sent).toEqual([]);
    t.emit({ type: 'state', state: st([{ id: 'g1', name: 'Billing', layout: {} }]) });
    expect(allTabs(deck.currentLayout.value)).toEqual([]);
  });

  it('a rejected save (Error) lets the next State win', () => {
    const { t, deck, st } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.selectGroup('g1');
    deck.openSession('a');
    vi.advanceTimersByTime(300);
    t.emit({ type: 'error', message: 'layout is larger than 64 KB' });
    t.emit({ type: 'state', state: st([{ id: 'g1', name: 'Billing', layout: {} }]) });
    expect(allTabs(deck.currentLayout.value)).toEqual([]);
  });

  it('openSession from the sidebar wakes; activateTab does not', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'asleep'), sess('b', 'p1', 'asleep')]);
    deck.selectProject('p1');
    deck.activateTab('a');
    expect(t.sent).toEqual([]);
    expect(deck.openTabs['p1']).toEqual(['a']);
    deck.openSession('b');
    expect(t.sent).toEqual([{ type: 'wake', deck_id: 'b' }]);
    deck.openSession('a', { wake: false });
    expect(t.sent).toHaveLength(1);
    deck.wakeSession('a');
    expect(t.sent[1]).toEqual({ type: 'wake', deck_id: 'a' });
  });

  it('openSession in a group activates the existing tab where it is', () => {
    const { deck } = setupGroups(
      [sess('a', 'p1', 'idle'), sess('b', 'p2', 'idle')],
      [{ id: 'g1', name: 'G', layout: split(pane('L', [s_('a')]), pane('R', [s_('b')])) }],
    );
    deck.selectGroup('g1');
    deck.focusPane('L');
    deck.openSession('b'); // present in R: focuses R, does not move it
    expect(deck.focusedPaneId.value).toBe('R');
    expect(panes(deck.currentLayout.value).map((p) => p.tabs.length)).toEqual([1, 1]);
  });

  it('openSession in a group adds an absent session to the focused pane and stays in the group', () => {
    const { deck } = setupGroups([sess('a', 'p1', 'idle'), sess('c', 'p2', 'idle')]);
    deck.selectGroup('g1');
    deck.openSession('a');
    deck.openSession('c');
    expect(allTabs(deck.currentLayout.value)).toEqual([s_('a'), s_('c')]);
    expect(deck.selected.value).toEqual({ kind: 'group', id: 'g1' });
  });

  it('newSession adds the new session to the project view focused pane', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.selectProject('p1');
    deck.openSession('a');
    deck.newSession('p1');
    t.emit({ type: 'session_changed', session: sess('n', 'p1', 'idle') });
    expect(deck.openTabs['p1']).toEqual(['a', 'n']);
    expect(deck.activeTab['p1']).toBe('n');
  });

  it('splitFocused moves the active tab to a new pane and focuses it; setRatio and setActiveTab persist', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'idle'), sess('b', 'p1', 'idle')]);
    deck.selectProject('p1');
    deck.openSession('a');
    deck.openSession('b');
    deck.splitFocused('column');
    const lay = deck.currentLayout.value;
    expect(lay).toMatchObject({ type: 'split', dir: 'column' });
    expect(panes(lay)).toHaveLength(2);
    expect(deck.focusedPaneId.value).toBe(panes(lay)[1]!.id);
    deck.setRatio([], 0.99);
    expect((deck.currentLayout.value as { ratio: number }).ratio).toBe(0.85);
    deck.setActiveTab(panes(lay)[0]!.id, 0);
    expect(deck.focusedPaneId.value).toBe(panes(lay)[0]!.id);
    vi.advanceTimersByTime(300);
    expect(t.sent.filter((r) => r.type === 'set_project_layout')).toHaveLength(1);
  });

  it('closeTab(key) closes in the selected view; legacy closeTab(project, deck) in the project view', () => {
    const { deck } = setupGroups([sess('a', 'p1', 'idle'), sess('b', 'p1', 'idle')]);
    deck.selectProject('p1');
    deck.openSession('a');
    deck.openSession('b');
    deck.closeTab('session:a');
    expect(deck.openTabs['p1']).toEqual(['b']);
    deck.closeTab('p1', 'b');
    expect(deck.openTabs['p1']).toEqual([]);
  });

  it('closeSession removes the tab from every layout and persists them', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.selectProject('p1');
    deck.openSession('a');
    deck.addToGroup('g1', 'a');
    vi.advanceTimersByTime(300);
    t.sent.length = 0;
    deck.closeSession('a');
    expect(allTabs(deck.layoutOf({ kind: 'group', id: 'g1' }))).toEqual([]);
    expect(deck.openTabs['p1']).toEqual([]);
    vi.advanceTimersByTime(300);
    expect(t.sent.map((r) => r.type).sort()).toEqual(['close_session', 'set_group_layout', 'set_project_layout']);
  });

  it('showChanges adds a Changes tab to the project view; activeTab reads "changes"', () => {
    const { t, deck } = setupGroups([sess('a', 'p1', 'idle')]);
    deck.selectProject('p1');
    deck.openSession('a');
    deck.showChanges('p1');
    expect(deck.activeTab['p1']).toBe('changes');
    expect(deck.openTabs['p1']).toEqual(['a']);
    expect(t.sent).toEqual([{ type: 'git_status', project_id: 'p1' }]);
    deck.openSession('a');
    expect(deck.activeTab['p1']).toBe('a');
  });

  it('compat: openTabs and activeTab are undefined for unknown projects', () => {
    const { deck } = setupGroups([]);
    expect(deck.openTabs['zzz']).toBeUndefined();
    expect(deck.activeTab['p1']).toBeUndefined();
  });
});
