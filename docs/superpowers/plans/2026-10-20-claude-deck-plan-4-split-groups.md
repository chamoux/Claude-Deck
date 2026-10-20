# Claude Deck Plan 4: Split Panes and Groups — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Tasks in the same **wave** touch different files and run **in parallel**. UI tasks must invoke `ui-ux-pro-max`, `tailwindcss-development` and `vue-best-practices`, and follow `app/design/MASTER.md`.

**Goal:** Show up to 4 sessions side by side (drag-to-split, like VS Code). Save cross-project combinations as named groups. Persist every layout.

**Architecture:**
- A pure, immutable TypeScript layout tree (`app/src/layout.ts`) holds all split, move and prune logic. It has full unit tests.
- The helper stores layouts as opaque JSON (`Project.layout`, `SavedState.groups`) behind new protocol requests.
- The store (`useDeck`) replaces `openTabs`/`activeTab` with per-view layouts.
- New components render the tree recursively with the existing `Splitter`, `TerminalView`, `ChangesTab` and HTML5 drag and drop.

**Spec:** `docs/superpowers/specs/2026-10-20-claude-deck-split-groups-design.md`.

## House rules

These are unchanged from Plans 2 and 3:
- **Never commit or push.** Stage only.
- **Package manager:** pnpm only.
- **Copies and target dirs:** each Rust agent works in a private repo copy with its own `CARGO_TARGET_DIR=/c/tmp/deck-target-<task>`, `-j 2`, and removes `app/src-tauri` from the copy's workspace members.
- **UI agents** work in the real repo, but only on the files they own. They run only their own tests plus `pnpm vue-tsc --noEmit`.
- **Do not edit `tauri.conf.json`.** The installed app and the dev setup both depend on it.

## Waves

| Wave | Tasks |
|---|---|
| 1 | T1 Rust (store, messages, core) · T2 `layout.ts` |
| 2 | T3 store + protocol mirror (needs T1 JSON + T2 API) |
| 3 | T4 layout components (LayoutView, PaneView, PaneTabs, DropOverlay, AsleepPanel) · T5 sidebar Groups and "Add to group" and drag sources |
| 4 | T6 App integration, shortcuts, terminal panel follows focus, demo scenes, screenshots · T7 review |

---

### T1 (wave 1): Rust — saved layouts and groups

**Owns:**
- `crates/deck-core/src/store.rs`
- `crates/deck-proto/src/messages.rs`
- `crates/deck-host/src/host/core.rs`
- `crates/deck-host/src/host/core_tests.rs`

**store.rs (TDD)**
```rust
pub const MAX_LAYOUT_BYTES: usize = 64 * 1024;

// in Project:
#[serde(default)]
pub layout: Option<serde_json::Value>,

// in SavedState:
#[serde(default)]
pub groups: Vec<Group>,

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group { pub id: String, pub name: String, #[serde(default = "empty_object")] pub layout: serde_json::Value }

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError { #[error("layout must be a JSON object")] NotObject, #[error("layout is larger than 64 KB")] TooLarge, #[error("unknown id")] Unknown }

impl SavedState {
    pub fn set_project_layout(&mut self, project_id: &str, layout: Option<serde_json::Value>) -> Result<(), LayoutError>;
    pub fn add_group(&mut self, name: &str) -> &Group;                 // id = new_id(); name trimmed, empty → "Group N" (N = groups.len()+1)
    pub fn rename_group(&mut self, group_id: &str, name: &str) -> Result<(), LayoutError>;  // empty name ignored (keeps old)
    pub fn remove_group(&mut self, group_id: &str) -> Result<(), LayoutError>;
    pub fn set_group_layout(&mut self, group_id: &str, layout: serde_json::Value) -> Result<(), LayoutError>;
}
```
`validate_layout(&Value)`: the value must be an object, and `serde_json::to_vec(v).len()` must be at most `MAX_LAYOUT_BYTES`.

Tests:
- an old file without these fields loads with `None` and `[]`;
- round trip with a project layout and 2 groups;
- an empty group name becomes "Group 1", then "Group 2";
- a non-object layout gives NotObject;
- a 70 KB layout gives TooLarge;
- an unknown id gives Unknown.

**messages.rs**
- Add these `Request` variants:
  - `SetProjectLayout{project_id, layout: Option<Value>}`
  - `AddGroup{name}`
  - `RenameGroup{group_id, name}`
  - `RemoveGroup{group_id}`
  - `SetGroupLayout{group_id, layout: Value}`
- `StateView` gains `#[serde(default)] groups: Vec<deck_core::store::Group>`.
- Exact-JSON tests:
  - `{"type":"add_group","name":"Billing"}`
  - `{"type":"set_group_layout","group_id":"g","layout":{"type":"pane","id":"p1","tabs":[],"active":0}}`
- Round trip for every new variant.

**core.rs**
- Each new request calls the store method.
- On `Ok`: `Save`, then broadcast `State`.
- On `Err`: send `Event::Error` with the error text to that client.
- `StateView` carries `groups` from saved state.
- `Project.layout` already travels inside `projects`.
- Tests:
  - each request saves and broadcasts;
  - `SetGroupLayout` with an array gives Error;
  - `RemoveGroup` of an unknown id gives Error;
  - `AddGroup` appears in `State.groups`.

**Finish**
- `cargo test -p deck-core -p deck-proto -p deck-host` green.
- clippy and fmt clean.
- Copy back and stage.

---

### T2 (wave 1): `app/src/layout.ts` — the pure layout tree

**Owns:** `app/src/layout.ts`, `app/src/layout.test.ts`.

```ts
export type TabRef = { kind: 'session'; deck_id: string } | { kind: 'changes'; project_id: string };
export interface Pane { type: 'pane'; id: string; tabs: TabRef[]; active: number }
export interface Split { type: 'split'; dir: 'row' | 'column'; ratio: number; a: LayoutNode; b: LayoutNode }
export type LayoutNode = Pane | Split;
export type Edge = 'left' | 'right' | 'top' | 'bottom';
export type DropZone = Edge | 'center';
export type Path = ('a' | 'b')[];
export const MAX_PANES = 4;
export const MIN_RATIO = 0.15;
export const MAX_RATIO = 0.85;

export function tabKey(t: TabRef): string;                         // 'session:<id>' | 'changes:<id>'
export function emptyLayout(newId: () => string): Pane;            // { type:'pane', id:newId(), tabs:[], active:0 }
export function panes(n: LayoutNode): Pane[];                      // left-to-right, top-to-bottom order
export function findPane(n: LayoutNode, paneId: string): Pane | undefined;
export function findTab(n: LayoutNode, key: string): { pane: Pane; index: number } | undefined;
export function allTabs(n: LayoutNode): TabRef[];
/** True when dropping `tab` on `zone` of pane `targetPaneId` is allowed:
 *  edge zones need panes(n).length < MAX_PANES unless the tab's own pane would empty and disappear
 *  (moving the only tab of a pane to another pane's edge keeps the count); 'center' is always allowed
 *  unless the target pane is the tab's own pane (no-op). */
export function canDrop(n: LayoutNode, targetPaneId: string, zone: DropZone, tab: TabRef): boolean;
/** Moves `tab` (removing it from wherever it is) to the target. Edge: the target pane is replaced by a Split
 *  (left/right → 'row', top/bottom → 'column', ratio 0.5) whose new pane holds only `tab`, placed first for
 *  left/top and second for right/bottom. Center: appended to the target pane and made active.
 *  Then empty panes are collapsed (their sibling takes the Split's place). Returns n unchanged if !canDrop. */
export function dropTab(n: LayoutNode, targetPaneId: string, zone: DropZone, tab: TabRef, newId: () => string): LayoutNode;
/** Adds `tab` to pane `paneId` (moving it if present elsewhere) and activates it. */
export function addTab(n: LayoutNode, paneId: string, tab: TabRef): LayoutNode;
/** Removes the tab with `key`; fixes `active`; collapses an emptied pane unless it is the only pane. */
export function closeTab(n: LayoutNode, key: string): LayoutNode;
export function setActive(n: LayoutNode, paneId: string, index: number): LayoutNode;  // clamped
export function setRatio(n: LayoutNode, path: Path, ratio: number): LayoutNode;         // clamped to [MIN_RATIO, MAX_RATIO]
export function prune(n: LayoutNode, keep: (t: TabRef) => boolean): LayoutNode;          // remove tabs failing keep, collapse
export function splitPane(n: LayoutNode, paneId: string, dir: 'row' | 'column', newId: () => string): LayoutNode;
   // "Split right/down" from the menu: moves the pane's ACTIVE tab into a new pane after it; no-op if 1 tab or at cap
export function neighbour(n: LayoutNode, paneId: string, dir: 'left' | 'right' | 'up' | 'down'): string | undefined;
   // geometric: compute pane rectangles in a unit square from ratios; nearest pane in that direction
export function normalize(raw: unknown, newId: () => string): LayoutNode;
   // validates shape deeply; drops unknown tab kinds; dedupes repeated tab keys (first wins); fixes active;
   // clamps ratios; caps panes at MAX_PANES (extra panes' tabs merged into the last kept pane); invalid → emptyLayout
```
All functions are pure and return new objects; never mutate their input. The test suite must include an immutability check.

Tests, at least these:
- `tabKey`.
- `dropTab`:
  - to each of the 4 edges, checking dir and order;
  - to the centre;
  - moving a tab from pane A to B's centre, which collapses A when it empties.
- The cap:
  - a 4th split allowed, a 5th refused;
  - the edge-move of a lone tab allowed at the cap.
- `canDrop` on the tab's own pane centre is false.
- `closeTab`, with active-fix and collapse; the last pane remains.
- `setRatio` clamps.
- `prune` removes and collapses.
- `splitPane`.
- `neighbour` in a 2×2 layout.
- `normalize`:
  - garbage gives empty;
  - duplicates removed;
  - 6 panes capped to 4, tabs kept;
  - bad ratio clamped.
- Round trip through `JSON.parse(JSON.stringify())`.

---

### T3 (wave 2): store and protocol mirror

**Owns:** `app/src/protocol.ts`, `app/src/useDeck.ts`, `app/src/useDeck.test.ts`, `app/src/protocol.test.ts`.

1. **Protocol mirror:** mirror T1 in `protocol.ts`: the 5 requests, `Group { id, name, layout }`, `Project.layout?: LayoutNode | null`, and `StateView.groups`. Use `LayoutNode` from `layout.ts`.
2. **Views:**
   - `type ViewRef = { kind: 'project'; id: string } | { kind: 'group'; id: string }`
   - `selected: Ref<ViewRef | null>`, replacing `selectedProjectId`. Keep `selectedProjectId` as a computed that is null for groups, so existing components keep working.
   - `focusedPaneId: Ref<string | null>`
   - `layoutOf(view): LayoutNode`, built from:
     - a project: `normalize(project.layout)`, or the default empty pane;
     - a group: `normalize(group.layout)`.
   - Each is pruned against existing sessions. A project view keeps only that project's sessions and its own Changes tab.
3. **Remove `openTabs`/`activeTab`.**
   - Derive what components need from the selected layout.
   - Keep compatibility helpers for other components:
     - `openTabs[projectId]` becomes the computed list of session ids in the project's own layout;
     - `activeTab[projectId]` becomes the focused pane's active tab of the project view.
   - If keeping them is messy, update their callers. List every changed caller.
4. **Actions.** Every one updates the in-memory layout immediately and persists through `SetProjectLayout`/`SetGroupLayout`, debounced 300 ms per view:
   - `selectProject(id)`, `selectGroup(id)`
   - `openSession(deckId)`: in a project view it adds to the focused pane and does what Wake did before. In a group view, if the session is in the group it activates it; if not, it adds it to the focused pane.
   - `newSession(projectId)` adds the new session to the project view's focused pane.
   - `dropTab(paneId, zone, tab)`, `closeTab(key)`, `setActiveTab(paneId, index)`, `setRatio(path, ratio)`, `splitFocused(dir)`, `focusPane(id)`, `moveFocus(dir)`
   - `addToGroup(groupId, deckId)`, `newGroup(name?)`, `renameGroup`, `removeGroup` (asks first through `confirm`)
   - `wakeSession(deckId)`, used by the asleep panel
   - `canDropHere(paneId, zone, tab)`: calls `layout.canDrop`, plus the project-only rule (a session from another project is refused in a project view)
5. **Pruning on server changes:** when a session or project disappears from State, prune every layout and persist only those that changed. Never persist on a pure load.
6. **Terminal panel follows focus.** Add `focusedProjectId` (computed):
   - in a project view, that project;
   - in a group, the project of the focused pane's active session.
7. **Tests (fake transport):**
   - selecting a group;
   - dropTab persists `SetGroupLayout` after the debounce (fake timers);
   - the project-only refusal;
   - pruning on SessionChanged/State removal;
   - `focusedProjectId` in a group;
   - addToGroup;
   - newGroup leading to `AddGroup`;
   - removeGroup confirm;
   - the openTabs/activeTab compatibility, or the updated callers.

---

### T4 (wave 3): layout components

**Owns:**
- `app/src/components/LayoutView.vue`
- `PaneView.vue`
- `PaneTabs.vue`
- `DropOverlay.vue`
- `AsleepPanel.vue`
- tests for all of the above

**Components:**
- `LayoutView` (prop `node`, `path`) renders recursively:
  - a split: flex row or column, a `Splitter` between (existing API: `drag`, `end`, `reset`), sizes from `ratio`, resize through `deck.setRatio(path, …)`;
  - a pane: `PaneView`.
- `PaneView` (prop `pane`):
  - a `PaneTabs` strip;
  - a body that renders the active tab, which is one of:
    - `TerminalView` for an awake session, keyed by target;
    - `AsleepPanel` for an asleep session;
    - `ChangesTab` for a changes tab.
  - Click or focus sets `focusPane`. The focused pane gets the accent top line (MASTER).
  - Min size 320×160 (CSS min-width/min-height).
- `PaneTabs` is like the current `SessionTabs`, but for one pane:
  - tabs are `draggable`, and `dragstart` sets `application/x-deck-tab` to the JSON TabRef;
  - in a group view it shows the project name faint after the session name;
  - right-click on a tab: Split right, Split down, Close tab, Move to new group (optional, only if trivial);
  - the `+` button creates a new session (project view only; in a group it is hidden).
- `DropOverlay` sits over the pane body while a drag with `application/x-deck-tab` or `application/x-deck-session` is over the pane.
  - It computes the zone from pointer position: the 25% edges are the edge zones, the rest is the centre.
  - It shows the resulting area as `bg-deck-accent/15` plus a 2 px accent border, only when `deck.canDropHere` allows it. A refused zone shows nothing. One exception: when the refusal comes from the project-only rule (a session from another project dragged over a project view), show a small centred hint chip, "Add sessions from other projects to a group". `canDropHere` returns a reason for this, `'other-project'`.
  - On drop it calls `deck.dropTab`.
  - Sidebar session drags carry `application/x-deck-session` with `{deck_id}` and are converted to a TabRef.
- `AsleepPanel` is centred:
  - lucide `Moon` icon, the session name, "This session is asleep", and a primary **Wake** button that calls `deck.wakeSession`;
  - Needs-you or error states if any.

**Tests:**
- recursive rendering of a 2×2;
- the splitter calls setRatio;
- PaneTabs dragstart data;
- DropOverlay zone math (mock `getBoundingClientRect`): highlight only when allowed, and drop calls dropTab;
- AsleepPanel Wake;
- focus on click.

Retire `SessionTabs.vue` (delete it and its test) once PaneTabs covers it, and list it in the report. Its only caller, App.vue, is updated in T6. If App.vue still needs it until T6, keep it and let T6 delete it.

---

### T5 (wave 3): sidebar Groups and drag sources

**Owns:** `ProjectTree.vue`, `SessionRow.vue`, `ContextMenu.vue` and their tests, plus a new `GroupList.vue` and its test.

- **`GroupList`:**
  - A "GROUPS" section label with a `Plus` button ("New group", which calls `deck.newGroup()`), above PROJECTS.
  - Rows are 30 px: lucide `LayoutGrid`, the name (inline rename on double-click or menu), an amber Needs-you count across its sessions, and the session count in faint mono.
  - The active group uses the active-row style.
  - Right-click: Rename, Delete group (through confirm).
  - Empty state: one faint line, "Drag sessions here or use + to combine projects".
  - Dropping a sidebar session onto a group row calls `addToGroup`.
- **`SessionRow`:**
  - `draggable="true"`; `dragstart` sets `application/x-deck-session` to `{"deck_id":…}` with `effectAllowed='move'`.
  - The menu gains **Add to group ▸**, a submenu listing groups plus **New group…**. If submenus are not supported in ContextMenu, add simple nested-submenu support there with keyboard Right/Left.
- `ProjectTree` mounts `GroupList` at the top. Selection now uses `deck.selected`.

**Tests:** create, rename and delete flows; drop onto a group row; the drag data on SessionRow; the Add-to-group submenu; keyboard navigation in the submenu.

---

### T6 (wave 4): App integration

**Owns:** `app/src/App.vue`, `app/src/App.test.ts`, `app/src/demo.ts`, `app/scripts/screenshots.mjs`.

**App.vue:**
- The middle area renders `<LayoutView :node="deck.currentLayout" :path="[]"/>` for the selected view.
- The ShellPanel uses `deck.focusedProjectId`.
- Empty states remain: no projects, and an empty project view.
- New: an empty group view, "Drag sessions here from the sidebar".

**Shortcuts:**
- **Ctrl+\\** splits right (`splitFocused('row')`).
- **Ctrl+Alt+←/→/↑/↓** moves focus.
- **Ctrl+Tab** cycles tabs in the focused pane.
- **Ctrl+W** closes the focused tab (outside terminals only, as before).
- Ctrl+1..9 still selects projects.
- Keep the terminal-focus rules from Plan 3.

**Demo and screenshots:**
- Add `split` (a project with 2 panes side by side) and `groups` (a 2×2 group: CodeHive ×2, NetWatch, diorama; one pane asleep; the Needs-you pane visible) scenes, plus a `dragging` scene that shows the overlay. Fake a dragover via script if possible, or skip it and say so.
- Add them to the screenshot script.
- Run the full UI suite, `vue-tsc` and `vite build`.

### T7 (wave 4): review

An opus review of T1–T6 against the spec. The controller then takes screenshots and does a visual check, and runs the full Rust and UI suites.
