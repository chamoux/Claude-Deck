import { computed, inject, provide, reactive, ref, shallowReactive, watch } from 'vue';
import type { ComputedRef, InjectionKey, Ref } from 'vue';
import {
  addTab as layoutAddTab,
  addTabTiled,
  allTabs,
  canDrop,
  closeTab as layoutCloseTab,
  dropTab as layoutDropTab,
  emptyLayout,
  findPane,
  findTab,
  neighbour,
  normalize,
  panes,
  prune,
  setActive,
  setRatio as layoutSetRatio,
  splitPane,
  tabKey,
  tileAll,
} from './layout';
import type { DropZone, LayoutNode, Path, TabRef } from './layout';
import { targetKey } from './protocol';
import type {
  Event,
  GitResult,
  Group,
  PastSession,
  Project,
  Request,
  SessionView,
  Settings,
  ShellPanel,
  StateView,
  Target,
} from './protocol';
import type { Connection, Transport } from './transport';

export interface TerminalSink {
  reset(): void;
  write(data: Uint8Array): void;
}

/** What the middle area shows: one project's own layout, or a saved group's layout. */
export type ViewRef = { kind: 'project'; id: string } | { kind: 'group'; id: string };

export type DropCheck = { ok: true } | { ok: false; reason: 'cap' | 'duplicate' | 'other-project' | 'noop' };

/** Debounce for persisting a layout edit, per view. */
export const LAYOUT_SAVE_DELAY_MS = 300;
/** How long after our own save a differing server layout is treated as stale rather than adopted. */
const ECHO_WINDOW_MS = 3000;

const viewKey = (v: ViewRef): string => `${v.kind}:${v.id}`;

export interface Deck {
  state: Ref<StateView | null>;
  connection: Ref<Connection>;
  memory: Ref<{ awake: number; bytes: number } | null>;
  lastError: Ref<string | null>;
  /** The view shown in the middle area. */
  selected: Ref<ViewRef | null>;
  /** Compatibility: the selected project, null while a group is shown. Assigning selects that project. */
  selectedProjectId: Ref<string | null>;
  focusedPaneId: Ref<string | null>;
  /** The project the project terminal follows: the project view, or the project of the focused pane's active session in a group. */
  focusedProjectId: ComputedRef<string | null>;
  /** Layout of the selected view (an empty pane when nothing is selected). */
  currentLayout: ComputedRef<LayoutNode>;
  groups: ComputedRef<Group[]>;
  /** Compatibility (read-only): session ids in a project's OWN layout. */
  openTabs: Record<string, string[]>;
  /** Compatibility (read-only): the active tab of a project's own layout ('changes' or a deck id). */
  activeTab: Record<string, string>;
  changes: Record<string, GitResult>;
  pastSessions: Record<string, PastSession[]>;
  confirm: Ref<null | { message: string; yes: () => void }>;
  closeDialogOpen: Ref<boolean>;
  projects: ComputedRef<Project[]>;
  sessionsOf(projectId: string): SessionView[];
  session(deckId: string): SessionView | undefined;
  shellRunning(projectId: string): boolean;
  needsYouCount: ComputedRef<number>;
  needsYouByProject: ComputedRef<Record<string, number>>;
  busySessions: ComputedRef<SessionView[]>;
  windowTitle: ComputedRef<string>;
  start(): void;
  stop(): void;
  selectProject(id: string): void;
  selectGroup(id: string): void;
  layoutOf(view: ViewRef): LayoutNode;
  /**
   * Shows a session: activates its tab if the selected view has it, else adds it to the focused pane
   * (a project view of another project, or no selection, switches to the session's project first).
   * Wakes an asleep session unless `{ wake: false }`. The sidebar uses the default (wake); tabs use `activateTab`.
   */
  openSession(deckId: string, opts?: { wake?: boolean }): void;
  /** Like `openSession` but never wakes: the pane shows the AsleepPanel instead. */
  activateTab(deckId: string): void;
  dropTab(paneId: string, zone: DropZone, tab: TabRef): void;
  setActiveTab(paneId: string, index: number): void;
  setRatio(path: Path, ratio: number): void;
  splitFocused(dir: 'row' | 'column'): void;
  focusPane(paneId: string): void;
  moveFocus(dir: 'left' | 'right' | 'up' | 'down'): void;
  addToGroup(groupId: string, deckId: string): void;
  tileGroup(groupId: string): void;
  /** Creates a group (the helper names it "Group N" when `name` is empty) and selects it once it arrives; optionally adds a session to it. */
  newGroup(name?: string, opts?: { addSession?: string }): void;
  renameGroup(groupId: string, name: string): void;
  removeGroup(groupId: string): void;
  wakeSession(deckId: string): void;
  canDropHere(paneId: string, zone: DropZone, tab: TabRef): DropCheck;
  newSession(projectId: string): void;
  sleep(deckId: string): void;
  closeSession(deckId: string): void;
  rename(deckId: string, name: string): void;
  /** Closes a tab (by key, e.g. 'session:a') in the selected view. The two-argument form is the legacy per-project call. */
  closeTab(key: string): void;
  closeTab(projectId: string, deckId: string): void;
  showChanges(projectId: string): void;
  refreshChanges(projectId: string): void;
  listPast(projectId: string): void;
  openPast(projectId: string, conversationId: string): void;
  addProject(path: string): void;
  removeProject(projectId: string): void;
  setShellPanel(projectId: string, panel: ShellPanel): void;
  closeShell(projectId: string): void;
  setSettings(s: Settings): void;
  attach(target: Target, rows: number, cols: number): void;
  detach(target: Target): void;
  resize(target: Target, rows: number, cols: number): void;
  input(target: Target, data: Uint8Array): void;
  subscribeTerminal(target: Target, sink: TerminalSink): () => void;
  requestClose(): void;
  /** Close dialog answer: keepRunning true keeps the helper and sessions running. */
  finishClose(keepRunning: boolean): void;
  confirmYes(): void;
  confirmNo(): void;
  /** Sets the native window title and optionally flashes the taskbar button. */
  attention(title: string, flash: boolean): void;
  /** Bumps each time the connection returns after a drop; mounted terminals must re-attach. */
  reattachGeneration: Ref<number>;
}

function isBusy(s: SessionView): boolean {
  return s.state === 'working' || s.state === 'needs_you';
}

export function createDeck(transport: Transport): Deck {
  const state = ref<StateView | null>(null);
  const connection = ref<Connection>('connected');
  const memory = ref<{ awake: number; bytes: number } | null>(null);
  const lastError = ref<string | null>(null);
  const selected = ref<ViewRef | null>(null);
  const focusedPaneId = ref<string | null>(null);
  const changes = reactive<Record<string, GitResult>>({});
  const pastSessions = reactive<Record<string, PastSession[]>>({});
  const confirm = ref<null | { message: string; yes: () => void }>(null);
  const closeDialogOpen = ref(false);
  const reattachGeneration = ref(0);

  // Terminal plumbing (not reactive): index <-> target key, and sinks per target key.
  const keyByIndex = new Map<number, string>();
  const sinks = new Map<string, Set<TerminalSink>>();
  const pendingNew = new Set<string>();
  let pendingGroup: { addSession?: string } | null = null;
  let unsubs: (() => void)[] = [];

  const projects = computed(() => state.value?.projects ?? []);
  const groups = computed(() => state.value?.groups ?? []);
  const sessions = computed(() => state.value?.sessions ?? []);
  const needsYouCount = computed(() => sessions.value.filter((s) => s.state === 'needs_you').length);
  const needsYouByProject = computed(() => {
    const out: Record<string, number> = {};
    for (const s of sessions.value) {
      if (s.state === 'needs_you') out[s.project_id] = (out[s.project_id] ?? 0) + 1;
    }
    return out;
  });
  const busySessions = computed(() => sessions.value.filter(isBusy));
  const windowTitle = computed(() => {
    const n = needsYouCount.value;
    if (n === 0) return 'Claude Deck';
    return `Claude Deck (${n} ${n === 1 ? 'needs' : 'need'} you)`;
  });

  function send(req: Request): void {
    void transport.send(req);
  }

  function sessionsOf(projectId: string): SessionView[] {
    return sessions.value.filter((s) => s.project_id === projectId);
  }
  function session(deckId: string): SessionView | undefined {
    return sessions.value.find((s) => s.deck_id === deckId);
  }
  function shellRunning(projectId: string): boolean {
    return state.value?.shells.find((s) => s.project_id === projectId)?.running ?? false;
  }

  // ---- Layouts -------------------------------------------------------------------------------
  // Each view keeps a local working copy of its layout. Edits change the copy at once and are saved
  // (debounced) with SetProjectLayout / SetGroupLayout. A State broadcast never overwrites a view
  // that has a save pending, or one we saved a moment ago whose echo has not come back yet.

  let idCounter = 0;
  const idPrefix = Math.random().toString(36).slice(2, 6);
  const newId = (): string => `n${idPrefix}${(++idCounter).toString(36)}`;

  const working = shallowReactive<Record<string, LayoutNode>>({});
  const fallback = new Map<string, LayoutNode>(); // stable empty layouts for views not adopted yet
  const dirty = new Map<string, number>(); // key -> time of the last local edit (save pending)
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const awaiting = new Map<string, { json: string; at: number }>(); // saved, echo not seen yet
  const focusByView = new Map<string, string>();
  const EMPTY = emptyLayout(newId);

  function layoutOf(view: ViewRef): LayoutNode {
    const key = viewKey(view);
    const w = working[key];
    if (w) return w;
    let f = fallback.get(key);
    if (!f) {
      f = emptyLayout(newId);
      fallback.set(key, f);
    }
    return f;
  }

  const currentLayout = computed<LayoutNode>(() => (selected.value ? layoutOf(selected.value) : EMPTY));

  const json = (n: LayoutNode): string => JSON.stringify(n);

  function keepFor(view: ViewRef): (t: TabRef) => boolean {
    return (t) => {
      if (t.kind === 'session') {
        const s = session(t.deck_id);
        return s !== undefined && (view.kind === 'group' || s.project_id === view.id);
      }
      return projects.value.some((p) => p.id === t.project_id) && (view.kind === 'group' || t.project_id === view.id);
    };
  }

  function persist(view: ViewRef): void {
    const key = viewKey(view);
    dirty.delete(key);
    timers.delete(key);
    const layout = working[key];
    if (!layout) return;
    const exists =
      view.kind === 'project' ? projects.value.some((p) => p.id === view.id) : groups.value.some((g) => g.id === view.id);
    if (!exists) return;
    if (view.kind === 'project') send({ type: 'set_project_layout', project_id: view.id, layout });
    else send({ type: 'set_group_layout', group_id: view.id, layout });
    awaiting.set(key, { json: json(normalize(layout, newId)), at: Date.now() });
  }

  function markDirty(view: ViewRef): void {
    const key = viewKey(view);
    dirty.set(key, Date.now());
    const old = timers.get(key);
    if (old !== undefined) clearTimeout(old);
    timers.set(key, setTimeout(() => persist(view), LAYOUT_SAVE_DELAY_MS));
  }

  function dropPendingSave(key: string): void {
    const t = timers.get(key);
    if (t !== undefined) clearTimeout(t);
    timers.delete(key);
    dirty.delete(key);
    awaiting.delete(key);
  }

  /** Applies `fn` to a view's layout; saves when the result differs. Returns whether it changed. */
  function edit(view: ViewRef, fn: (n: LayoutNode) => LayoutNode): boolean {
    const cur = layoutOf(view);
    const next = fn(cur);
    if (next === cur || json(next) === json(cur)) return false;
    working[viewKey(view)] = next;
    markDirty(view);
    return true;
  }

  const isSelected = (view: ViewRef): boolean => selected.value !== null && viewKey(selected.value) === viewKey(view);

  /** The pane new tabs go to: the focused pane of the shown view, else the view's first pane. */
  function targetPaneId(view: ViewRef): string | undefined {
    const lay = layoutOf(view);
    if (isSelected(view) && focusedPaneId.value && findPane(lay, focusedPaneId.value)) return focusedPaneId.value;
    return panes(lay)[0]?.id;
  }

  function focusPane(paneId: string): void {
    const view = selected.value;
    if (!view || !findPane(currentLayout.value, paneId)) return;
    focusedPaneId.value = paneId;
    focusByView.set(viewKey(view), paneId);
  }

  function firstValidFocus(view: ViewRef): string | null {
    const lay = layoutOf(view);
    const remembered = focusByView.get(viewKey(view));
    return remembered && findPane(lay, remembered) ? remembered : (panes(lay)[0]?.id ?? null);
  }

  // Keep the focused pane valid when the layout or the selection changes.
  watch(
    [selected, currentLayout],
    () => {
      const view = selected.value;
      if (!view) {
        focusedPaneId.value = null;
        return;
      }
      if (focusedPaneId.value && findPane(currentLayout.value, focusedPaneId.value)) return;
      focusedPaneId.value = firstValidFocus(view);
    },
    { flush: 'sync' },
  );

  function selectView(view: ViewRef): void {
    selected.value = view;
    focusedPaneId.value = firstValidFocus(view);
  }

  /** Puts `tab` in view: activates it where it is, else adds it to the target pane. */
  function revealTab(view: ViewRef, tab: TabRef): void {
    const found = findTab(layoutOf(view), tabKey(tab));
    let paneId: string | undefined;
    if (found) {
      paneId = found.pane.id;
      edit(view, (n) => setActive(n, found.pane.id, found.index));
    } else {
      paneId = targetPaneId(view);
      if (paneId === undefined) return;
      const id = paneId;
      edit(view, (n) => layoutAddTab(n, id, tab));
    }
    if (isSelected(view)) focusPane(paneId);
  }

  /** Adopts the server's layouts (fromServer) and prunes tabs whose session or project is gone. */
  function reconcile(fromServer: boolean): void {
    const st = state.value;
    if (!st) return;
    const views: { view: ViewRef; raw: unknown }[] = [
      ...st.projects.map((p) => ({ view: { kind: 'project', id: p.id } as ViewRef, raw: p.layout })),
      ...(st.groups ?? []).map((g) => ({ view: { kind: 'group', id: g.id } as ViewRef, raw: g.layout })),
    ];
    const live = new Set(views.map((v) => viewKey(v.view)));
    for (const key of Object.keys(working)) {
      if (live.has(key)) continue;
      delete working[key];
      fallback.delete(key);
      focusByView.delete(key);
      dropPendingSave(key);
    }
    for (const { view, raw } of views) {
      const key = viewKey(view);
      if (fromServer && shouldAdopt(key, raw)) {
        const incoming = normalize(raw, newId);
        const cur = working[key];
        // Keep the current object when nothing differs (stable pane ids, no re-render, no save).
        const same = cur !== undefined && (json(cur) === json(incoming) || (allTabs(cur).length === 0 && allTabs(incoming).length === 0));
        if (!same) working[key] = incoming;
      }
      const cur = working[key];
      const base = cur ?? normalize(raw, newId);
      const pruned = prune(base, keepFor(view));
      if (!cur) {
        working[key] = pruned; // first sight of this view: a load, never saved
      } else if (json(pruned) !== json(cur)) {
        working[key] = pruned;
        markDirty(view);
      }
    }
    const sel = selected.value;
    if (sel && !live.has(viewKey(sel))) {
      const first = st.projects[0];
      if (first) selectView({ kind: 'project', id: first.id });
      else selected.value = null;
    }
  }

  /** Whether an incoming server layout for the view may replace the local copy. */
  function shouldAdopt(key: string, raw: unknown): boolean {
    if (timers.has(key)) return false; // a local edit is waiting to be saved
    const aw = awaiting.get(key);
    if (!aw) return true;
    if (json(normalize(raw, newId)) === aw.json) {
      awaiting.delete(key); // our own echo
      return false;
    }
    if (Date.now() - aw.at < ECHO_WINDOW_MS) return false; // stale broadcast from before our save landed
    awaiting.delete(key);
    return true;
  }

  function activeTabOfProject(projectId: string): string | undefined {
    const view: ViewRef = { kind: 'project', id: projectId };
    const lay = layoutOf(view);
    const focused = isSelected(view) && focusedPaneId.value ? findPane(lay, focusedPaneId.value) : undefined;
    const pane = focused ?? panes(lay)[0];
    const tab = pane?.tabs[pane.active];
    if (!tab) return undefined;
    return tab.kind === 'session' ? tab.deck_id : 'changes';
  }

  // Compatibility views over a project's OWN layout. The proxies read reactive state inside `get`,
  // so templates and computeds that use them stay reactive.
  const openTabs = new Proxy({} as Record<string, string[]>, {
    get(_t, prop) {
      if (typeof prop !== 'string' || !projects.value.some((p) => p.id === prop)) return undefined;
      return allTabs(layoutOf({ kind: 'project', id: prop })).flatMap((t) => (t.kind === 'session' ? [t.deck_id] : []));
    },
  });
  const activeTab = new Proxy({} as Record<string, string>, {
    get(_t, prop) {
      if (typeof prop !== 'string' || !projects.value.some((p) => p.id === prop)) return undefined;
      return activeTabOfProject(prop);
    },
  });

  const selectedProjectId = computed<string | null>({
    get: () => (selected.value?.kind === 'project' ? selected.value.id : null),
    set: (id) => {
      if (id !== null) selectView({ kind: 'project', id });
    },
  });

  const focusedProjectId = computed<string | null>(() => {
    const view = selected.value;
    if (!view) return null;
    if (view.kind === 'project') return view.id;
    const pane = focusedPaneId.value ? findPane(currentLayout.value, focusedPaneId.value) : undefined;
    const tab = pane?.tabs[pane.active];
    if (!tab) return null;
    return tab.kind === 'changes' ? tab.project_id : (session(tab.deck_id)?.project_id ?? null);
  });

  function closeTab(a: string, b?: string): void {
    if (b !== undefined) {
      edit({ kind: 'project', id: a }, (n) => layoutCloseTab(n, `session:${b}`));
      return;
    }
    if (selected.value) edit(selected.value, (n) => layoutCloseTab(n, a));
  }

  /** Removes a session's tab from every layout (used when the session is closed). */
  function closeEverywhere(deckId: string): void {
    for (const key of Object.keys(working)) {
      const view: ViewRef = { kind: key.startsWith('group:') ? 'group' : 'project', id: key.slice(key.indexOf(':') + 1) };
      edit(view, (n) => layoutCloseTab(n, `session:${deckId}`));
    }
  }

  function canDropHere(paneId: string, zone: DropZone, tab: TabRef): DropCheck {
    const view = selected.value;
    if (!view) return { ok: false, reason: 'noop' };
    if (view.kind === 'project') {
      const owner = tab.kind === 'session' ? session(tab.deck_id)?.project_id : tab.project_id;
      if (owner !== undefined && owner !== view.id) return { ok: false, reason: 'other-project' };
    }
    const lay = currentLayout.value;
    const target = findPane(lay, paneId);
    if (!target) return { ok: false, reason: 'noop' };
    if (zone === 'center' && target.tabs.some((t) => tabKey(t) === tabKey(tab))) return { ok: false, reason: 'duplicate' };
    if (!canDrop(lay, paneId, zone, tab)) return { ok: false, reason: 'cap' };
    if (json(layoutDropTab(lay, paneId, zone, tab, () => '__probe__')) === json(lay)) return { ok: false, reason: 'noop' };
    return { ok: true };
  }

  function dropTab(paneId: string, zone: DropZone, tab: TabRef): void {
    const view = selected.value;
    if (!view || !canDropHere(paneId, zone, tab).ok) return;
    if (edit(view, (n) => layoutDropTab(n, paneId, zone, tab, newId))) {
      const at = findTab(layoutOf(view), tabKey(tab));
      if (at) focusPane(at.pane.id);
    }
  }

  function splitFocused(dir: 'row' | 'column'): void {
    const view = selected.value;
    const id = focusedPaneId.value;
    if (!view || !id) return;
    const tab = findPane(layoutOf(view), id)?.tabs[findPane(layoutOf(view), id)?.active ?? 0];
    if (!tab) return;
    if (edit(view, (n) => splitPane(n, id, dir, newId))) {
      const at = findTab(layoutOf(view), tabKey(tab));
      if (at) focusPane(at.pane.id);
    }
  }

  function refreshChanges(projectId: string): void {
    send({ type: 'git_status', project_id: projectId });
  }

  /** True when the shown view has an active Changes tab for the project. */
  function changesVisible(projectId: string): boolean {
    return panes(currentLayout.value).some((p) => {
      const t = p.tabs[p.active];
      return t?.kind === 'changes' && t.project_id === projectId;
    });
  }

  function onSessionChanged(next: SessionView): void {
    const st = state.value;
    if (!st) return;
    const i = st.sessions.findIndex((s) => s.deck_id === next.deck_id);
    const prev = i >= 0 ? st.sessions[i] : undefined;
    if (i >= 0) st.sessions[i] = next;
    else st.sessions.push(next);

    reconcile(false);
    if (!prev && pendingNew.has(next.project_id)) {
      pendingNew.delete(next.project_id);
      revealTab({ kind: 'project', id: next.project_id }, { kind: 'session', deck_id: next.deck_id });
    }
    if (prev?.state === 'working' && next.state === 'idle' && changesVisible(next.project_id)) {
      refreshChanges(next.project_id);
    }
  }

  function onEvent(e: Event): void {
    switch (e.type) {
      case 'state': {
        const known = new Set(groups.value.map((g) => g.id));
        state.value = { ...e.state, groups: e.state.groups ?? [] };
        reconcile(true);
        if (pendingGroup) {
          const created = groups.value.find((g) => !known.has(g.id));
          if (created) {
            const { addSession } = pendingGroup;
            pendingGroup = null;
            selectView({ kind: 'group', id: created.id });
            if (addSession) addToGroup(created.id, addSession);
          }
        }
        break;
      }
      case 'session_changed':
        onSessionChanged(e.session);
        break;
      case 'shell_changed': {
        const st = state.value;
        if (!st) break;
        const i = st.shells.findIndex((s) => s.project_id === e.shell.project_id);
        if (i >= 0) st.shells[i] = e.shell;
        else st.shells.push(e.shell);
        break;
      }
      case 'attached': {
        const key = targetKey(e.target);
        keyByIndex.set(e.index, key);
        sinks.get(key)?.forEach((s) => s.reset());
        break;
      }
      case 'detached': {
        const key = targetKey(e.target);
        for (const [index, k] of keyByIndex) if (k === key) keyByIndex.delete(index);
        break;
      }
      case 'memory':
        memory.value = { awake: e.awake, bytes: e.bytes };
        break;
      case 'git_status':
        changes[e.project_id] = e.result;
        break;
      case 'past_sessions':
        pastSessions[e.project_id] = e.sessions;
        break;
      case 'needs_confirm': {
        const { action } = e;
        askConfirm(e.message, () => {
          switch (action.type) {
            case 'sleep':
              send({ type: 'sleep', deck_id: action.deck_id, force: true });
              break;
            case 'close_session':
              send({ type: 'close_session', deck_id: action.deck_id, force: true });
              break;
            case 'remove_project':
              send({ type: 'remove_project', project_id: action.project_id, force: true });
              break;
          }
        });
        break;
      }
      case 'error':
        lastError.value = e.message;
        awaiting.clear(); // a rejected layout must not hide the server's truth
        pendingGroup = null;
        break;
    }
  }

  function onOutput(index: number, data: Uint8Array): void {
    const key = keyByIndex.get(index);
    if (key === undefined) return;
    sinks.get(key)?.forEach((s) => s.write(data));
  }

  function indexOf(target: Target): number | undefined {
    const key = targetKey(target);
    for (const [index, k] of keyByIndex) if (k === key) return index;
    return undefined;
  }

  function askConfirm(message: string, yes: () => void): void {
    confirm.value = { message, yes };
  }

  function start(): void {
    stop();
    unsubs = [
      transport.onEvent(onEvent),
      transport.onOutput(onOutput),
      transport.onConnection((c) => {
        const wasDown = connection.value !== 'connected';
        connection.value = c;
        if (c === 'connected' && wasDown) {
          // The helper may have restarted: terminal indices are reassigned, so forget them all
          // and let every mounted terminal re-attach for a fresh snapshot.
          keyByIndex.clear();
          reattachGeneration.value++;
          // Re-adopt the server's layouts: drop local pending saves; the next State replaces them.
          for (const key of [...timers.keys(), ...awaiting.keys()]) dropPendingSave(key);
        }
      }),
      transport.onCloseRequested(requestClose),
    ];
    send({ type: 'hello', client: 'window' });
  }

  function stop(): void {
    unsubs.forEach((u) => u());
    unsubs = [];
  }

  function selectProject(id: string): void {
    selectView({ kind: 'project', id });
  }

  function selectGroup(id: string): void {
    selectView({ kind: 'group', id });
  }

  function openSession(deckId: string, opts: { wake?: boolean } = {}): void {
    const s = session(deckId);
    if (!s) return;
    let view = selected.value;
    if (!view || (view.kind === 'project' && view.id !== s.project_id)) {
      view = { kind: 'project', id: s.project_id };
      selectView(view);
    }
    revealTab(view, { kind: 'session', deck_id: deckId });
    if (s.state === 'asleep' && (opts.wake ?? true)) send({ type: 'wake', deck_id: deckId });
  }

  function addToGroup(groupId: string, deckId: string): void {
    if (!session(deckId) || !groups.value.some((g) => g.id === groupId)) return;
    const view: ViewRef = { kind: 'group', id: groupId };
    if (findTab(layoutOf(view), `session:${deckId}`)) return;
    edit(view, (n) => addTabTiled(n, { kind: 'session', deck_id: deckId }, newId));
  }

  function tileGroup(groupId: string): void {
    if (!groups.value.some((g) => g.id === groupId)) return;
    edit({ kind: 'group', id: groupId }, (n) => tileAll(n, newId));
  }

  function newGroup(name = '', opts: { addSession?: string } = {}): void {
    pendingGroup = opts;
    send({ type: 'add_group', name });
  }

  function removeGroup(groupId: string): void {
    const g = groups.value.find((x) => x.id === groupId);
    if (!g) return;
    askConfirm(`Delete group "${g.name}"? Its sessions keep running.`, () =>
      send({ type: 'remove_group', group_id: groupId }),
    );
  }

  function showChanges(projectId: string): void {
    const view: ViewRef = selected.value?.kind === 'group' ? selected.value : { kind: 'project', id: projectId };
    revealTab(view, { kind: 'changes', project_id: projectId });
    refreshChanges(projectId);
  }

  function moveFocus(dir: 'left' | 'right' | 'up' | 'down'): void {
    if (!focusedPaneId.value) return;
    const next = neighbour(currentLayout.value, focusedPaneId.value, dir);
    if (next) focusPane(next);
  }

  function newSession(projectId: string): void {
    pendingNew.add(projectId);
    send({ type: 'new_session', project_id: projectId });
  }

  // The helper decides whether Claude is busy and answers with `needs_confirm`.
  function sleep(deckId: string): void {
    const s = session(deckId);
    if (!s || s.state === 'asleep') return;
    send({ type: 'sleep', deck_id: deckId, force: false });
  }

  function closeSession(deckId: string): void {
    const s = session(deckId);
    if (!s) return;
    send({ type: 'close_session', deck_id: deckId, force: false });
    closeEverywhere(deckId);
  }

  function removeProject(projectId: string): void {
    send({ type: 'remove_project', project_id: projectId, force: false });
  }

  function requestClose(): void {
    if (busySessions.value.length === 0) void transport.close(false);
    else closeDialogOpen.value = true;
  }

  function finishClose(keepRunning: boolean): void {
    closeDialogOpen.value = false;
    void transport.close(keepRunning);
  }

  function confirmYes(): void {
    const c = confirm.value;
    confirm.value = null;
    c?.yes();
  }

  return {
    state,
    connection,
    memory,
    lastError,
    selected,
    selectedProjectId,
    focusedPaneId,
    focusedProjectId,
    currentLayout,
    groups,
    openTabs,
    activeTab,
    changes,
    pastSessions,
    confirm,
    closeDialogOpen,
    reattachGeneration,
    projects,
    sessionsOf,
    session,
    shellRunning,
    needsYouCount,
    needsYouByProject,
    busySessions,
    windowTitle,
    start,
    stop,
    selectProject,
    selectGroup,
    layoutOf,
    openSession,
    activateTab: (deckId) => openSession(deckId, { wake: false }),
    dropTab,
    setActiveTab: (paneId, index) => {
      if (selected.value) edit(selected.value, (n) => setActive(n, paneId, index));
      focusPane(paneId);
    },
    setRatio: (path, ratio) => {
      if (selected.value) edit(selected.value, (n) => layoutSetRatio(n, path, ratio));
    },
    splitFocused,
    focusPane,
    moveFocus,
    addToGroup,
    tileGroup,
    newGroup,
    renameGroup: (groupId, name) => send({ type: 'rename_group', group_id: groupId, name }),
    removeGroup,
    wakeSession: (deckId) => send({ type: 'wake', deck_id: deckId }),
    canDropHere,
    newSession,
    sleep,
    closeSession,
    rename: (deckId, name) => send({ type: 'rename', deck_id: deckId, name }),
    closeTab,
    showChanges,
    refreshChanges,
    listPast: (projectId) => send({ type: 'list_past_sessions', project_id: projectId }),
    openPast: (projectId, conversationId) =>
      send({ type: 'open_past_session', project_id: projectId, conversation_id: conversationId }),
    addProject: (path) => send({ type: 'add_project', path }),
    removeProject,
    setShellPanel: (projectId, panel) => send({ type: 'set_shell_panel', project_id: projectId, panel }),
    closeShell: (projectId) => send({ type: 'close_shell', project_id: projectId }),
    setSettings: (settings) => send({ type: 'set_settings', settings }),
    attach: (target, rows, cols) => send({ type: 'attach', target, rows, cols }),
    detach: (target) => send({ type: 'detach', target }),
    resize: (target, rows, cols) => send({ type: 'resize', target, rows, cols }),
    input(target, data) {
      const index = indexOf(target);
      if (index !== undefined) void transport.input(index, data);
    },
    subscribeTerminal(target, sink) {
      const key = targetKey(target);
      const set = sinks.get(key) ?? new Set<TerminalSink>();
      set.add(sink);
      sinks.set(key, set);
      return () => {
        set.delete(sink);
      };
    },
    requestClose,
    finishClose,
    confirmYes,
    attention: (title, flash) => void transport.attention(title, flash),
    confirmNo() {
      confirm.value = null;
    },
  };
}

export const deckKey: InjectionKey<Deck> = Symbol('deck');

export function provideDeck(deck: Deck): void {
  provide(deckKey, deck);
}

export function useDeck(): Deck {
  const deck = inject(deckKey);
  if (!deck) throw new Error('useDeck() called without provideDeck()');
  return deck;
}
