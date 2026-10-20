# Claude Deck: split panes and groups (design spec)

Date: 2026-10-20
Status: approved in chat, awaiting written-spec review
Builds on `2026-10-19-claude-deck-design.md` and `app/design/MASTER.md`.

## 1. Purpose

Show several Claude sessions side by side, as VS Code editor groups do, and save combinations of sessions from different projects as named **groups**. Example: "Billing: CodeHive + NetWatch".

## 2. Decisions taken in chat

| Topic | Decision |
|---|---|
| Split | Drag a tab onto the terminal area. A blue overlay shows the drop target: the left, right, top or bottom half creates a new pane, and the centre moves the tab into that pane. The same is available from the tab's right-click menu (**Split right**, **Split down**) and with **Ctrl+\\** (split right). |
| Panes | At most 4 per view. Each pane has its own tab strip. The focused pane shows the blue top line. Dividers can be dragged. Closing a pane's last tab removes the pane. |
| Cross-project | Allowed only in **groups**. A project view splits only that project's own sessions. |
| Groups | A **Groups** section sits in the sidebar above Projects. **+** creates "Group N", which can be renamed inline. Clicking a group opens its saved layout. Sessions from any project are added by dragging from the sidebar or a tab, or with right-click on a session → **Add to group ▸ <name>**. |
| Tabs in groups | Each tab also shows its project name in faint text. |
| Terminal panel | Follows the project of the focused pane. |
| Asleep sessions | Opening a group or a project never wakes sessions on its own. A pane whose session is asleep shows a **Wake** button. Clicking a session in the sidebar still wakes it, as before. |
| Persistence | Layouts are saved: each project's own layout, and each group's layout. |

## 3. Approaches considered

1. **Own layout tree (chosen).** A small recursive split tree, with at most 4 leaves, rendered by our own Vue components with the existing `Splitter` and HTML5 drag and drop.
   - It stays small and matches MASTER.md styling.
   - It needs no new dependency.
2. **A docking library (dockview-vue).** It has richer features, such as floating panes and tab overflow. It is heavier, its styling clashes with MASTER.md, and it far exceeds a 4-pane limit.
3. **A fixed grid with presets** (1, 2 side by side, 2 stacked, 2×2). It is simplest, but it gives no drag-to-split, which is what the user asked for.

## 4. Model

### 4.1 Layout tree (window side, TypeScript; stored by the helper as opaque JSON)

```ts
type TabRef = { kind: 'session'; deck_id: string } | { kind: 'changes'; project_id: string };
type Pane = { type: 'pane'; id: string; tabs: TabRef[]; active: number };
type Split = { type: 'split'; dir: 'row' | 'column'; ratio: number /* 0.15–0.85 */; a: Node; b: Node };
type Node = Pane | Split;
```

Rules:
- At most 4 panes in a tree.
- A session appears **at most once per view**. A PTY has a single size, so dragging a tab *moves* it and never copies it.
- In a project view, only that project's sessions and Changes tab are allowed. In a group, any project's sessions and Changes tabs are allowed.
- **Pruning:** when a session is closed or its project removed, its TabRef is dropped everywhere. A pane left empty is removed, and its sibling takes its place.

### 4.2 Saved state (helper, `deck-core::store`)

- `Project.layout: Option<serde_json::Value>`, with serde default `None`. `None` means one pane holding the project's open tabs, which is today's behaviour.
- `SavedState.groups: Vec<Group>` with `Group { id, name, layout: serde_json::Value }`, with serde default empty.
- The helper treats layouts as **opaque**. It validates only that each is under 64 KB and is a JSON object. The window owns the layout logic and the pruning.
- **Open tabs move into the layout.** The tabs that are open per project live in the layout's tab lists. The window's in-memory `openTabs`/`activeTab` are replaced by the layout, so tabs now survive restarts.

### 4.3 Protocol (`deck-proto::messages`)

New `Request`s:
- `SetProjectLayout { project_id, layout }`
- `AddGroup { name }`, which replies with a `State` that contains the group
- `RenameGroup { group_id, name }`
- `RemoveGroup { group_id }`
- `SetGroupLayout { group_id, layout }`

`StateView` gains `groups: Vec<Group>`, and `Project` gains `layout`. Every change triggers `Save` and a `State` broadcast.

## 5. Window behaviour

- **Selection:** the sidebar selects either a project or a group. The middle area renders that view's layout tree.
- **Drop targets:**
  - While a tab is dragged, each pane shows 5 zones: left, right, top and bottom quarters (split), and the centre (move).
  - The zone under the pointer gets a translucent accent overlay (`bg-deck-accent/15` with a 2 px `deck-accent` border) showing the resulting area.
  - Splits are refused when they would make a 5th pane. The overlay then shows the centre zone only.
  - A drag from the sidebar works the same way: dropping a session creates or fills a pane. Dropping a session from another project onto a *project* view is refused, with the hint "Add sessions from other projects to a group".
- **Focus:** clicking inside a pane focuses it, shows the accent top line, and makes the terminal panel follow that pane's project. Ctrl+Tab cycles tabs within the focused pane, and Ctrl+Alt+Left/Right moves focus between panes.
- **Resize:** each split has a `Splitter`. The ratio is clamped to 0.15–0.85, and a pane is at least 320 px wide or 160 px tall. Changes are saved, debounced by 300 ms.
- **Asleep:** a pane whose active tab is an asleep session shows a centred panel with the session name, "This session is asleep", and a **Wake** button (primary). Only after waking does its TerminalView attach.
- **Groups section:**
  - **+ New group.**
  - Rows show a lucide `LayoutGrid` icon, the name, a Needs-you badge (the count across its sessions), and the number of sessions.
  - Right-click: Rename, Delete group. Deleting asks first; it never closes or stops sessions.
- **Session context menu:** gains **Add to group ▸** (each group, plus **New group…**), which adds the session as a tab in the group's focused pane. If the tree has no pane yet, one is created.
- **Memory:** each awake visible pane is a running Claude, about 600 MB. The footer already shows the total. The 4-pane cap is the guard.

## 6. Error handling

- An invalid or oversized layout from disk is replaced by the default layout, and the window logs a warning. It never crashes.
- A layout that references a missing session is pruned when it loads.
- A drop that would break a rule (5th pane, duplicate session, cross-project in a project view) is refused visually: the zone does not highlight.

## 7. Testing

- **Pure TypeScript layout module (`layout.ts`), with unit tests:**
  - split at each edge and centre-move;
  - the 4-pane cap;
  - no duplicates;
  - pruning and collapsing;
  - ratio clamping;
  - the project-only rule;
  - serialisation round trip.
- **deck-core store tests:** layouts and groups save and load, old files load with defaults, and the 64 KB cap is enforced.
- **Core tests:** the new requests save and broadcast State.
- **Component tests:**
  - drop-zone highlighting and refusal;
  - the Wake panel;
  - the Groups sidebar;
  - "Add to group".
- **Demo mode:** add a `groups` scene with a 2×2 group and a `split` scene. Take screenshots for review.

## 8. Out of scope

- Floating or undocked panes.
- More than 4 panes.
- The same session shown in two panes.
- Splitting the project terminal panel.
