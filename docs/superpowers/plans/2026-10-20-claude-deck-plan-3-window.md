# Claude Deck Plan 3: The Window — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax. Tasks in one **wave** touch different files and run **in parallel**. UI tasks must invoke the `ui-ux-pro-max` skill (user's primary UI skill) and `tailwindcss-development` (Tailwind v4) before writing components, and `vue-best-practices` for Vue.

**Goal:** Build the Tauri 2 window: layout A (project tree + nested sessions, session tabs + Changes tab, Claude terminal) with the right-hand project terminal panel. It talks to `deck-host` over the named pipe, shows Needs you loudly, asks before closing, and ships as a per-user installer with the chosen icon.

**Architecture:**
- **`app/src-tauri`** (Rust) is a thin bridge. It:
  - starts `deck-host.exe` if it is not running, connects to the pipe, and forwards frames both ways;
  - turns Control frames into a webview event and Output frames into a Tauri `Channel`;
  - intercepts window close;
  - sets the window title and flashes the taskbar.
- **`app/src`** (Vue 3 `<script setup>` + TypeScript + Tailwind v4 + xterm.js) holds all UI state in one store, `useDeck`, fed only by protocol events. All UI logic is unit-tested with Vitest against a fake transport, with no Tauri and no helper needed.

**Tech stack (versions from the registry, 2026-10-20):** tauri 2.x (`@tauri-apps/cli` 2.12.3, `@tauri-apps/api` 2.12.4), Vue 3.5.44, Vite 8.3.4, Vitest 5.0.5, Tailwind CSS 4.3.3 with `@tailwindcss/vite`, `@xterm/xterm` 6.0.0, `@xterm/addon-fit` 0.11.0, TypeScript, pnpm.

**Inputs:**
- Spec §5, §4.4, §4.5, §6, §8.
- `crates/deck-proto/src/messages.rs`, the wire format. Its tests pin the exact JSON.
- `assets/icon.svg`, icon B "session dots".
- The approved mockups:
  - colours: sidebar `#1d2027`, terminal background `#16181d`, borders `#2a2e37`, active row `#3b5bdb33`, accent `#3b5bdb`;
  - dots: Working `#40c057`, Idle `#4dabf7`, Needs you `#fab005`, Asleep a hollow ring `#8a90a0`;
  - change badges: C `#8ce99a` on `#40c05733`, M `#ffc078` on `#f59f0033`, D `#ffa8a8` on `#fa525233`, R `#a5d8ff` on `#74c0fc33`.

## House rules

- **Never run `git commit` or `git push`.** Stage only.
- **pnpm only** (user rule). Use `pnpm add`, `pnpm dlx`, `pnpm create`, never npm/npx/yarn. If a scaffolder writes `package-lock.json`, delete it. Approve build scripts with `pnpm approve-builds` when asked (esbuild).
- **Tailwind v4 CSS-first:** `@import "tailwindcss";` plus `@theme` tokens, with no `tailwind.config.js`.
- **RAM:** building the Tauri Rust side compiles wry/webview2 and is heavy. Only Task 1 (scaffold) and Task 2 (bridge) build `src-tauri`. UI tasks run only `pnpm vitest` and `pnpm vite build`. Rust commands use `-j 2` and the agent's own `CARGO_TARGET_DIR=/c/tmp/deck-target-<task>`.
- **Parallel safety:** each UI task owns its own files (see the table) and works in the real repo, because Vue files don't share a build-breaking compile. Run only that task's tests (`pnpm vitest run <pattern>`). Never edit another task's files. Shared types live only in `app/src/protocol.ts` (Task 3).
- TypeScript is `strict`. No `any` in shipped code.

## Waves

| Wave | Tasks |
|---|---|
| 0 | Task 1: scaffold (Tauri + Vue + TS + Tailwind v4 + xterm + Vitest), workspace wiring, icons |
| 1 | Task 2 bridge (Rust) · Task 3 protocol types + `useDeck` store · Task 4 TerminalView · Task 5 sidebar · Task 6 Changes tab + past-sessions dialog · Task 7 shell panel + splitter · Task 8 dialogs + footer |
| 2 | Task 9: App shell integration, Needs you title/flash wiring, keyboard shortcuts |
| 3 | Task 10: installer (NSIS, per user, deck-host bundled) · Task 11: manual run with real Claude + final review |

## File structure

| File | Task | Responsibility |
|---|---|---|
| `app/package.json`, `vite.config.ts`, `tsconfig*.json`, `index.html`, `src/main.ts`, `src/style.css` | 1 | Scaffold; `style.css` = `@import "tailwindcss"` + `@theme` tokens above |
| `app/src-tauri/Cargo.toml`, `tauri.conf.json`, `capabilities/default.json`, `icons/*` | 1 | Tauri config; icons generated from `assets/icon.svg` |
| `app/src-tauri/src/main.rs`, `lib.rs` | 1 / 2 | App entry / bridge |
| `app/src-tauri/src/bridge.rs` | 2 | Helper launch, pipe client, reader/writer tasks, reconnect |
| `app/src/protocol.ts` | 3 | TS mirror of `messages.rs` (Request, Event, views, Target) |
| `app/src/transport.ts` | 3 | `Transport` interface + `tauriTransport()` (invoke/listen/Channel) + `fakeTransport()` for tests |
| `app/src/useDeck.ts` + `useDeck.test.ts` | 3 | Single store: state view, selection, open tabs, attach indices, Needs you count, memory, errors |
| `app/src/components/TerminalView.vue` (+ test) | 4 | One xterm per target; Attached→reset; Output→write; onData→input; fit + resize |
| `app/src/components/ProjectTree.vue`, `SessionRow.vue`, `ContextMenu.vue` (+ tests) | 5 | Sidebar layout A, dots, badges, right-click menus |
| `app/src/components/ChangesTab.vue`, `PastSessionsDialog.vue` (+ tests) | 6 | Changes list grouped C/M/D/R; resume-past list |
| `app/src/components/ShellPanel.vue`, `Splitter.vue` (+ tests) | 7 | Right project terminal panel, minimize to rail, width |
| `app/src/components/CloseDialog.vue`, `SettingsDialog.vue`, `ConfirmDialog.vue`, `StatusFooter.vue` (+ tests) | 8 | Close prompt, settings, "Claude may be working" confirm, memory footer |
| `app/src/App.vue`, `app/src/components/SessionTabs.vue` | 9 | Layout composition, tabs, shortcuts, title/flash calls |
| `app/src-tauri/tauri.conf.json` (bundle section), `scripts/build-installer.ps1` | 10 | NSIS per-user installer with deck-host.exe |

---

### Task 1 (wave 0): Scaffold

- [ ] `pnpm create tauri-app app --template vue-ts --manager pnpm --identifier lb.code.claudedeck --yes`, or the equivalent non-interactive flags of create-tauri-app 4.7. Delete any `package-lock.json`.
- [ ] Add the dependencies:
  - `pnpm add @xterm/xterm @xterm/addon-fit`
  - `pnpm add -D tailwindcss @tailwindcss/vite vitest @vue/test-utils jsdom`
- [ ] Vite: add the `tailwindcss()` plugin. Vitest: `environment: 'jsdom'`.
- [ ] `src/style.css`: `@import "tailwindcss";` plus `@theme` with the colour tokens above (`--color-deck-sidebar`, `--color-deck-term`, `--color-deck-border`, `--color-deck-accent`, `--color-dot-working`, `--color-dot-idle`, `--color-dot-needs`, `--color-dot-asleep`, and the four change-badge colour pairs).
- [ ] Add `app/src-tauri` to the root Cargo `members`. Its Cargo.toml depends on `deck-host` (for `pipe`, `framed`, `locate`), `deck-proto`, `tauri` 2, `serde_json` and `tokio`.
- [ ] Set `tauri.conf.json`:
  - `productName` "Claude Deck", one window 1400×900 (min 900×560), title "Claude Deck"
  - `withGlobalTauri: false`
  - CSP `default-src 'self'; style-src 'self' 'unsafe-inline'` (xterm needs inline styles)
- [ ] Icons: `pnpm tauri icon ../assets/icon.svg`. If the SVG is not accepted, rasterize it first to a 1024×1024 PNG with `resvg` via `pnpm dlx @resvg/resvg-js-cli`, or with any working rasterizer, and say which.
- [ ] Check: `pnpm vite build` succeeds, `pnpm vitest run` runs with 0 tests, and `cargo build -p claude-deck -j 2` succeeds (this is the heavy one, so run it once).
- [ ] Add `app/node_modules/`, `app/dist/` and `app/src-tauri/target/` to `.gitignore`. Stage `app/` (excluding the ignored folders) and `.gitignore`.

### Task 2 (wave 1): Bridge (`app/src-tauri/src/bridge.rs`, `lib.rs`)

Rust side, with no UI logic. Commands:

```rust
#[tauri::command] async fn deck_send(json: String, state: State<'_, Bridge>) -> Result<(), String>;      // Control frame
#[tauri::command] async fn deck_input(index: u32, data: Vec<u8>, state: State<'_, Bridge>) -> Result<(), String>; // Input frame
#[tauri::command] async fn deck_subscribe(output: tauri::ipc::Channel<OutputChunk>, state: State<'_, Bridge>) -> Result<(), String>;
#[tauri::command] async fn deck_close(keep_running: bool, app: AppHandle, state: State<'_, Bridge>) -> Result<(), String>; // WindowClosing then exit
#[tauri::command] fn deck_attention(title: String, flash: bool, window: Window);  // set title; flash = request_user_attention(Informational) if not focused
#[derive(Serialize, Clone)] struct OutputChunk { index: u32, data: Vec<u8> }
```

Events emitted to the webview:
- `deck:event` with the Event JSON string;
- `deck:connection` with `"connected" | "reconnecting" | "failed"`;
- `deck:close-requested` (no payload).

Behaviour:
- **Launching the helper:**
  - find `deck-host.exe` next to `current_exe()`; in dev, use `CARGO_TARGET_DIR`'s or the workspace's `target/debug/deck-host.exe`;
  - spawn `deck-host serve` with `CREATE_NO_WINDOW | DETACHED_PROCESS`;
  - connect with `deck_host::pipe::connect_client(pipe_name(sid), 5 s)`, launching only if the first connect fails.
- **Handshake:** send `Hello{Window}` first.
- **Reader task:** decodes frames. Control → `emit("deck:event", json)`. Output → `channel.send(OutputChunk)`, buffered until `deck_subscribe` is called.
- **Reconnect:** when the pipe closes unexpectedly, emit `reconnecting`, relaunch and reconnect with backoff (0.5, 1, 2, 4 s, then every 5 s). On success, emit `connected` and re-send Hello. After 30 s, emit `failed` and keep trying.
- **Window close:** in `on_window_event(CloseRequested)`, call `api.prevent_close()` and emit `deck:close-requested`. The UI decides and calls `deck_close`.
- **Tests:** Rust unit tests for helper-exe discovery (dev vs installed path) and for the frame routing function (Control vs Output), using a fake sink trait. Building the full Tauri app runs once at the end.

### Task 3 (wave 1): Protocol types, transport, store

- **`protocol.ts`:** TypeScript discriminated unions mirroring `messages.rs` exactly, including the `type`/`kind`/`status` tags and snake_case fields.
  - Mirror `SessionState` (`'working' | 'needs_you' | 'idle' | 'asleep'`), `ChangeKind`, `Project` (`shell_panel`), `Settings` (`auto_sleep_minutes: number | null`, `claude_path`, `window`, `shell: 'power_shell' | 'git_bash' | 'cmd'`).
  - **Check the exact serde names in the Rust tests.** `ShellKind::PowerShell` with `rename_all = "snake_case"` serialises as `"power_shell"`; confirm against Rust.
- **`transport.ts`:**
```ts
export interface Transport {
  send(req: Request): Promise<void>;
  input(index: number, data: Uint8Array): Promise<void>;
  onEvent(cb: (e: Event) => void): () => void;
  onOutput(cb: (index: number, data: Uint8Array) => void): () => void;
  onConnection(cb: (s: 'connected' | 'reconnecting' | 'failed') => void): () => void;
  onCloseRequested(cb: () => void): () => void;
  close(keepRunning: boolean): Promise<void>;
  attention(title: string, flash: boolean): Promise<void>;
}
export function tauriTransport(): Transport;   // invoke + listen + Channel
export function fakeTransport(): Transport & { sent: Request[]; emit(e: Event): void; emitOutput(i: number, d: Uint8Array): void };
```
- **`useDeck.ts`:** a single reactive store, created with `createDeck(transport)` and provided to the app. It exposes:
  - `state` (the latest StateView, patched by `SessionChanged` / `ShellChanged`);
  - `selectedProjectId`, `openTabs: Map<projectId, deckId[]>`, `activeTab: Map<projectId, deckId | 'changes'>`;
  - `indexOf(target)` (from `Attached`), `memory`, `lastError`, `connection`;
  - `needsYouCount` (computed), `needsYouByProject` (computed);
  - `changes: Map<projectId, GitResult>`, `pastSessions: Map<projectId, PastSession[]>`.
- **Actions**, one per Request:
  - `selectProject`, `openSession` (Wake if asleep, then Attach), `newSession`, `sleep(id, force)`, `closeSession(id, force)`, `rename`, `refreshChanges`, `listPast`, `openPast`, `addProject`, `removeProject`;
  - `setShellPanel`, `openShell`, `closeShell`, `setSettings`, `requestClose()` (decides between dialog and direct close).
- **Tests (Vitest, fake transport):**
  - State patching.
  - `needsYouCount`.
  - `openSession` on an asleep session sends Wake, then Attach after the `SessionChanged` (awake).
  - Error routing: an Error containing "Claude may be working" opens the confirm flow.
  - `requestClose` with no busy session calls `close(false)` directly, sleeping idle ones per the core rules. With a busy (Working/NeedsYou) session it sets `closeDialogOpen = true`.

### Task 4 (wave 1): `TerminalView.vue`

Props: `target: Target`. It uses `useDeck` and owns one `Terminal` from `@xterm/xterm` plus a `FitAddon`.
- On mount it calls `deck.attach(target, rows, cols)` after fitting.
- On `Attached` for its target, it calls `term.reset()`, and the following Output (the snapshot) and later Output frames are written with `term.write(data)`.
- `term.onData` → `transport.input(index, utf8)`.
- On a debounced container resize → fit → `Resize`.
- On unmount → `Detach` and `term.dispose()`.
- Theme: background `#16181d`, foreground `#d6d8dd`, cursor `#74c0fc`. Font: Cascadia Mono, Consolas, monospace, 13 px.
- Claude uses the alternate screen; nothing special is needed, because xterm handles it.
- **Tests (jsdom, mock xterm via `vi.mock('@xterm/xterm')`):**
  - attach is sent on mount;
  - `Attached`, then reset, then Output is written in order;
  - onData sends input with the right index;
  - unmount sends Detach.

### Task 5 (wave 1): Sidebar (`ProjectTree.vue`, `SessionRow.vue`, `ContextMenu.vue`)

Layout A, as in the approved mockup:
- **Header:** "PROJECTS" and "+ Add". Add uses the Tauri dialog plugin folder picker (`@tauri-apps/plugin-dialog`; add it with pnpm and to capabilities). If the plugin is missing, the store action accepts a path, and the picker is wired in Task 9.
- **Project row:** name, open-session count, amber badge with the Needs-you count (also when collapsed), and "Folder missing" state.
- **Session rows** nested under their project:
  - dot by state (Working is green with a soft pulse, Idle blue, Needs you amber, Asleep a hollow ring);
  - name, falling back to "New session";
  - "Can't resume" in red when `error` is set;
  - **Needs you rows** get an amber background (`#fab00522`), amber text and the words "Needs you".
- **Clicks:** clicking a session row opens it (wake if needed).
- **Right-click a project:** New Claude session · Resume past session… · Open in File Explorer (`@tauri-apps/plugin-opener` `openPath`) · Remove from list (confirm if it has awake sessions).
- **Right-click a session:** Rename (inline input) · Sleep now (disabled unless Idle, or confirm) · Close session.
- **Accessibility:** keyboard navigation with arrow keys and Enter, and focus rings.
- **Tests:** dots and labels per state; badge count; menu items call the right store actions; Sleep now confirms when not Idle.

### Task 6 (wave 1): `ChangesTab.vue`, `PastSessionsDialog.vue`

- **ChangesTab:**
  - Groups Created, Modified, Deleted, Renamed with counts. Each line shows a badge letter, the file name, then the folder dimmed. Renamed shows `old → new`. Conflicts get a "conflict" tag.
  - "and N more" when `total > items.length`.
  - States: "Not a git project", "Git isn't installed", and the error line.
  - A ↻ button calls `refreshChanges`.
  - Nothing is clickable.
- **PastSessionsDialog:** the list of past sessions, newest first: last prompt (or "No prompt"), plus the date in relative format ("3 days ago"). Clicking one calls `openPast`. Shows an empty state.
- **Tests:** grouping and counts, the cap text, each GitResult state, and the dialog calling `openPast`.

### Task 7 (wave 1): `ShellPanel.vue`, `Splitter.vue`

- Right-hand panel for the selected project's shell (spec §5).
- **Open:** a header ("Terminal — <project>", shell name, a minimize button, a close-shell button) above a `TerminalView` with `target {kind:'shell', project_id}`.
- **Minimized:** a 28 px rail on the right edge with a vertical "Terminal" label and an icon. One click reopens it.
- **Size:** width comes from `project.shell_panel.width`, adjustable with `Splitter`, which uses pointer events. The panel can't shrink the Claude area below 420 px. Width and open state are saved with `setShellPanel`, debounced 300 ms after drag end.
- **Shell lifecycle:** switching projects swaps to that project's shell. The shell starts on first open, through Attach. While minimized it stays running (the TerminalView is unmounted, but the helper keeps the shell). Closing the shell asks for confirmation.
- **Tests:** minimize/reopen sends `SetShellPanel`; dragging clamps the width; switching project swaps the target.

### Task 8 (wave 1): Dialogs and footer

- **CloseDialog:** "N sessions are still working." Buttons: **Keep running in background** (default, primary) · **Stop and close** · **Cancel**. They call `transport.close(true/false)` or dismiss. Below, a list of the busy session names.
- **ConfirmDialog:** generic. Used for "Claude may be working. Stop it anyway?", "Close this session?" and "Remove project?".
- **SettingsDialog:**
  - auto-sleep: a minutes number field, 5–240, plus an Off toggle;
  - claude.exe path (text plus a Browse button, through the dialog plugin, or text only);
  - shell select: PowerShell / Git Bash / cmd.
  - Save → `setSettings`.
- **StatusFooter:** "3 awake · 1.8 GB" (from `Memory`), the connection state ("Reconnecting…" in amber), and the last error, dismissable.
- **Tests:** button wiring, settings validation (clamp), footer formatting (bytes → GB/MB).

### Task 9 (wave 2): `App.vue` + `SessionTabs.vue` integration

- **Layout:**
  - left: ProjectTree (resizable 200–360 px);
  - middle: SessionTabs (open sessions of the selected project, then "Changes (N)"), above the active TerminalView or ChangesTab;
  - right: ShellPanel (open or minimized);
  - bottom: StatusFooter.
- **Needs you visibility** (spec §5):
  - tabs of Needs-you sessions show an amber dot and amber text;
  - the window title is `Claude Deck` or `Claude Deck (2 need you)`;
  - `attention(title, flash)` flashes only when `needsYouCount` rises.
- **Close:** `onCloseRequested` → `requestClose()`.
- **Reconnect** (safety, added 2026-10-20):
  - When the connection goes `reconnecting` → `connected`, the store clears all terminal indices and every mounted TerminalView re-attaches, with a fresh snapshot.
  - The bridge drops queued Input/Attach/Detach/Resize frames while it is disconnected, because indices are reassigned after a helper restart.
- **Keyboard shortcuts:**
  - Ctrl+T new session;
  - Ctrl+W close tab (detach only, the session keeps running);
  - Ctrl+Tab next tab;
  - Ctrl+` toggle the shell panel;
  - Ctrl+1..9 switch project.
- **Empty states:** no projects ("Add a project folder to start"), and a project with no sessions ("New Claude session" button).
- **Integration test:** a fake-transport app mount with a scripted event sequence: state → select → open session → Attached → output → needs_you → title call.

### Task 10 (wave 3): Installer

- Tauri bundle with NSIS, `installMode: "currentUser"`. Bundle `deck-host.exe` built in release as a resource next to the app exe, using `bundle.resources` or `externalBin`; choose `externalBin` with the target-triple naming Tauri requires.
- `scripts/build-installer.ps1`:
  - `cargo build -p deck-host --release`;
  - copy `deck-host.exe` to `app/src-tauri/binaries/deck-host-x86_64-pc-windows-msvc.exe`;
  - `pnpm tauri build`.
- Uninstall leaves `%LOCALAPPDATA%\ClaudeDeck` alone. Spec §9 mentions an opt-in "remove my settings" checkbox; skip it in v1 and note that in the spec.

### Task 11 (wave 3): Manual run and final review

- `pnpm tauri dev`, with real Claude. The user does this with the controller:
  - add 2 projects, 3 sessions;
  - Needs you flash on a permission prompt;
  - auto-sleep with the minutes set to 5;
  - wake;
  - Changes tab in a git project;
  - project terminal: `git status`, minimize/reopen;
  - close the window with a busy session → dialog → Keep running → reopen → still live.
- Measure RAM: the window process tree and the helper.
- Opus review of `app/`. Stage everything and update the memory checkpoint.
