# Claude Deck — design spec

Date: 2026-10-19
Status: approved in chat, awaiting written-spec review
Name: "Claude Deck" is a working name. Rename before the first build if you like.

## 1. Purpose

A light Windows app for running many Claude Code sessions, grouped by project, in one window. It exists for two reasons:

1. **Organisation.** Today there are 10–15 terminal tabs with Claude and no way to see which belongs to which project or what each one is doing.
2. **Memory.** Each Claude Code session reserves about 610 MB (about 270 MB actually in RAM), measured on this VM on 2026-10-19. Fifteen sessions filled the VM's memory and broke the Azure VPN. Claude Code is a Bun program and runs one session per process, so the only way to save memory is to close sessions nobody is using and bring them back on demand. The app does that automatically ("auto-sleep").

Success looks like: 10 sessions open across 4 projects, 2 in real use. After 30 minutes only those 2 Claude processes are still running, and every other session wakes on click with its full conversation.

What it is not: an editor, a file browser or a git client. It shows Claude's terminal, which files changed, and nothing else.

## 2. Decisions taken in chat

| Topic | Decision |
|---|---|
| Layout | Layout A: project tree on the left with sessions nested under each project; a terminal on the right with a "Changes" tab next to the session tabs |
| Right-click a project | New Claude session · Resume past session… · Open in File Explorer · Remove from list. No built-in file browser |
| Right-click a session | Rename · Sleep now · Close session |
| Session states | Working, Needs you, Idle, Asleep. Working and Needs you never auto-sleep |
| Auto-sleep | An Idle session sleeps after 30 minutes (a setting). Waking runs `claude --resume <id>` |
| Reopening the app | Every session not closed by the user comes back, like VS Code tabs. Sessions still running are shown live; the rest are asleep |
| Closing the window | Idle sessions sleep right away. If any are Working or Needs you: "Keep running in background / Stop and close / Cancel" |
| Changes tab | Uncommitted changes from `git status`: Created, Modified, Deleted (plus Renamed). A list only, nothing to open or click |
| Adding projects | An "Add project" button and a folder picker. Old Claude sessions for that folder are not imported; they appear under "Resume past session…" |
| Stack | Tauri 2 window (Rust + Vue 3 + TypeScript + Tailwind v4 + xterm.js, pnpm) and a separate Rust helper process that owns the sessions |
| Claude itself | Used as installed (Claude Code 2.1.303, Bun). No custom client, no Agent SDK |

## 3. Architecture

Two programs and one repo at `C:\Users\Chamoux\Desktop\claude-deck`.

```
Window (Tauri, ~100–150 MB, only while open)
   │  named pipe, current Windows user only
Helper: deck-host.exe (Rust, target ≤ 30 MB with 10 awake sessions)
   │  one ConPTY terminal per awake session
claude.exe   claude.exe   (asleep sessions have no process)
```

The window is only a viewer. The helper owns every Claude process, so sessions can keep running when the window is closed.

### 3.1 Repo layout

| Path | What it is |
|---|---|
| `crates/deck-core` | Pure logic, no I/O: session state machine and sleep rules, saved-state model, `git status` parser, Claude project-folder name encoding. All unit-tested |
| `crates/deck-proto` | Messages between window, helper and hook (types plus framing) |
| `crates/deck-host` | The helper binary: pipe server, ConPTY terminals (`portable-pty`), a headless terminal per session (`vt100`), the auto-sleep timer, git, and the `hook` subcommand |
| `app/` | The Tauri 2 window. `app/src-tauri` is a thin pipe client; `app/src` is the Vue UI |
| `tests/fake-claude` | A small test program that behaves like Claude for the integration tests (see §10) |

### 3.2 Helper (deck-host.exe)

- Started by the window if it is not already running, as a separate process that survives the window closing. Only one runs per user (a named mutex).
- Exits on its own when no session is awake and no window is connected.
- Holds, for each awake session: the Claude process, its ConPTY, a headless terminal with 5,000 lines of scrollback (so a reopened window can redraw the screen exactly), and the session's state. Project shells keep 1,000 lines, because a full 5,000-line scrollback measured about 19 MB.
- Each Claude session runs in a Windows **Job Object** with kill-on-close. Sleeping or killing a session therefore ends its whole process tree, including MCP servers and background tasks, and a helper crash takes the tree down with it. Project shells are not put in a job, so a GUI app the user starts from one (for example `code .`) outlives the shell. The job deliberately does not allow breakaway (`JOB_OBJECT_LIMIT_BREAKAWAY_OK`): Git Bash children would use it to escape. Keyboard input to a terminal is queued to a per-process writer thread, so a large paste never blocks the helper.
- Logs to `%LOCALAPPDATA%\ClaudeDeck\logs\` and keeps 7 days.
- Not a Windows service: it must run as the user so Claude can use the user's login.

### 3.3 Window (Tauri)

- On start it connects to the helper (starting it if needed) and draws the saved projects and sessions.
- Keeps one xterm.js view per awake session that has been opened, so switching tabs is instant. When the window opens, or a tab is opened for the first time, it asks the helper for a screen snapshot, then receives live output.
- Sends keyboard input and terminal size changes to the helper.
- Footer: "3 awake · 1.8 GB" (memory of all awake Claude processes, reported by the helper).
- UI built with Vue 3 `<script setup>` + TypeScript + Tailwind v4.

### 3.4 Pipe

- Name: `\\.\pipe\claude-deck-<user SID>`. Its ACL allows only the current user. No network ports.
- Framing: 1 byte type, 4 byte length, payload. Type 1 = JSON control message, type 2 = raw terminal output for one session (session index + bytes), type 3 = raw input.
- Window → helper: `get_state`, `add_project`, `remove_project`, `new_session`, `wake`, `sleep`, `close_session`, `rename`, `attach`, `detach`, `input`, `resize`, `git_status`, `list_past_sessions`, `set_settings`, `window_closing`.
- Helper → window: `state` (full), `session_changed`, `output`, `snapshot`, `memory`, `error`.
- Hook → helper: `hook_event { deck_id, payload }`. The payload is Claude's hook JSON, which carries the conversation id as `session_id`. The helper answers with the one-byte `HOOK_ACK` after applying the event (§4.1).

## 4. Sessions

### 4.1 Starting, waking, sleeping

| Action | What the helper runs, in the project folder |
|---|---|
| New session | `claude --session-id <new uuid> --settings <hooks file>` |
| Wake | `claude --resume <conversation id> --settings <hooks file>` (`--resume` at launch keeps the same id; confirmed by the spike) |
| Sleep | Send Ctrl+C twice (Claude's own exit), wait up to 5 s, then end the process tree. The conversation is already saved by Claude |

Every Claude process gets the environment variable `CLAUDE_DECK_SESSION=<deck id>`, a UUID owned by the app. That id never changes. Inside a running process, `/clear` and `/resume` switch Claude to another conversation with a new `session_id`. So the helper keys sessions by the deck id and stores the `session_id` of the latest `SessionStart` as the conversation to wake. For a new session, both ids start out equal (`--session-id <deck id>`).

The hooks file lives at `%LOCALAPPDATA%\ClaudeDeck\hooks.json`. The helper writes it at start. It registers `deck-host.exe hook` for the events in §4.2. Claude runs hook commands through a POSIX shell, even on Windows, and that shell strips backslashes. So the command path in hooks.json uses forward slashes (`C:/Users/...`); the spike confirmed this. The hook command works as follows:
- It reads Claude's hook JSON from standard input and reads `CLAUDE_DECK_SESSION`.
- It sends one message to the pipe carrying both ids.
- It waits up to 2 s for a one-byte ack, which the helper sends after applying the event, so events from one session are applied in order. It always exits 0.
- If the variable is missing (for example, Claude started outside the app), it exits silently.

hooks.json never sets `async`. The user's own `~/.claude/settings.json` is never changed.

### 4.2 States

| State | Dot | Meaning | Auto-sleep |
|---|---|---|---|
| Working | green | Claude is doing something | Never |
| Needs you | amber, plus a badge on the project | Claude asked a question or wants permission | Never: sleeping would lose the question |
| Idle | blue | Claude finished and waits for the next prompt | After 30 minutes (setting: 5–240 minutes, or off) |
| Asleep | hollow | No process. The conversation is on disk | — |

How the helper knows, from Claude's hooks:

| Hook | New state |
|---|---|
| `SessionStart` with source `startup` or `resume` | Idle (Working if background work is known) |
| `SessionStart` with source `clear`, `compact` or anything else | No change: auto-compaction can run in the middle of a turn |
| `UserPromptSubmit` | Working. Its `prompt` names the session if it has no name yet, except Claude's own `<task-notification>` prompts |
| `PreToolUse`, `PostToolUse` | Working (this also clears Needs you after permission is given) |
| `Notification` of type `permission_prompt`, or any unknown type | Needs you (also fires for questions asked with AskUserQuestion) |
| `Notification` of type `idle_prompt` ("Claude is waiting for your input", about 60 s after the last activity) | Working → Idle when no background work is known. Never clears Needs you; keeps an existing idle clock. This is the only signal after Esc on a permission prompt, which sends no hook at all (spike 2) |
| `Notification` of an informational type (`auth_success`, `elicitation_complete`, `elicitation_response`, `quota_auto_resume_fired`, `agent_completed`) | No change |
| `Stop` whose `background_tasks` and `session_crons` are both empty | Idle, and the idle clock starts |
| `Stop` that lists background tasks or crons | Working, until a later `Stop` lists none. Claude wakes itself when that work reports back, by sending a `UserPromptSubmit` whose prompt starts with `<task-notification>`. Sleeping the session would kill that work |
| `SessionEnd` | No change: `/clear` and `/resume` end a conversation but keep the process |
| The process exits | Asleep |
| Anything else (`SubagentStop`, `StopFailure`, …) | No change |

Keyboard input has two effects:
- In an Idle session it restarts the idle clock, so a half-typed prompt is not slept away while the user is typing.
- In a Needs you session it means the user answered, so the session moves to Working. The spike showed Claude sends no hook when a permission is approved; without this rule, the amber highlight would stay on until the approved tool finished.

Two more effects, from spike 2:
- If the answer was Esc (rejected), Claude sends nothing more. The following `idle_prompt` turns the session Idle.
- A new folder shows Claude's trust prompt, whose default is "No, exit". The app never answers it for the user. Until the user does, the new session counts as Working and is never slept.

**Safe default:** a session whose state the helper cannot confirm is treated as Working, and it is never auto-slept. Examples:
- No hook has arrived since it started.
- The user pressed Esc. The spike showed that neither `Stop` nor `idle_prompt` follows an interrupt, so the session stays Working until the next prompt.

"Sleep now" on such a session asks first: "Claude may be working. Stop it anyway?"

### 4.3 Names

A session is named after its first prompt, cut to 40 characters, until the user renames it. Renamed names are never overwritten.

### 4.4 Closing the window

1. Idle sessions are put to sleep at once.
2. If any session is Working or Needs you, a dialog appears: "N sessions are still working." with **Keep running in background** (the default button), **Stop and close**, and **Cancel**.
3. "Keep running" closes only the window. Those sessions continue in the helper, become Idle when done, and then follow the normal auto-sleep timer.
4. "Stop and close" puts every session to sleep, then closes.

### 4.5 Opening the app

- Every session not closed by the user is listed under its project.
- Sessions still running in the helper show live, exactly where they are.
- All others are asleep and wake on click (a few seconds).
- After a reboot or a helper crash, every session is asleep. Nothing saved is lost; work that was in progress was interrupted.

### 4.6 Closing a session

"Close session" (with a confirmation if it is Working or Needs you) stops the process and removes the session from the sidebar. Its conversation stays in Claude's history and can be reopened from "Resume past session…".

### 4.7 Resume past session…

Lists the Claude conversations for the project folder that are not open in the app, newest first, each with its last prompt and date. Source: `~/.claude/projects/<encoded folder>/*.jsonl`. The folder name is the project path with every character that is not an ASCII letter or digit replaced by `-`. Claude shortens paths over 200 characters with a hash, which v1 does not handle. Existing folders use both `C--` and `c--`, so matching ignores case, and the `cwd` field inside the file is checked to confirm the match.

## 5. Window and sidebar

- Left: **Projects** header with **+ Add**. Each project shows its name, a count of its open sessions, and an amber badge when a session needs you. Sessions are nested under their project with their state dot and name.
- Clicking a session opens or switches to its tab. Clicking an asleep session wakes it.
- **Needs you is hard to miss.** When Claude asks for permission or asks a question, that session's row in the sidebar and its tab turn amber with the words "Needs you", the project gets an amber badge with the count (also on a collapsed project), and the window title becomes "Claude Deck (1 needs you)". If the window is not in front, its taskbar button flashes once. All of these clear as soon as the session leaves Needs you.
- Middle: tabs for the selected project (its open sessions, then **Changes (N)**), and the Claude terminal.
- **Project terminal (right panel).** Requested 2026-10-20, so the user can `git commit`/`git push` without leaving the app.
  - It is a plain shell in the project folder, docked on the right of the Claude terminal, with a draggable splitter.
  - One shell per project. It starts the first time the panel is opened for that project. Switching projects switches the panel to that project's shell.
  - **Minimize** collapses the panel to a thin bar on the right edge, and one click reopens it. The shell keeps running while minimized, so its history and any running command stay.
  - The shell is PowerShell by default. A setting switches it to Git Bash or cmd.
  - The helper owns these shells too, through the same terminal code as Claude sessions. They never auto-sleep and are not part of "Needs you".
  - They close when the window closes, or with "Stop and close". With "Keep running in background" they also stay alive until the helper exits.
  - The panel's open or minimized state and its width are saved per project.
- Right-click menus as in §2. "Open in File Explorer" runs `explorer.exe <project folder>`. "Remove from list" asks first if the project has busy sessions (Working or Needs you), puts them to sleep, and removes the project from the app only. Files and Claude's history are not touched.
- Settings (a small dialog):
  - auto-sleep minutes
  - path to `claude.exe` (found automatically on PATH or `~/.local/bin`)
  - project terminal shell (PowerShell / Git Bash / cmd)

## 6. Changes tab

- Runs `git status --porcelain=v1 -z --untracked-files=all` in the project folder.
- Mapping: untracked, added or copied → **Created**; modified or type change in either column → **Modified**; deleted → **Deleted**; renamed → **Renamed** (shown as old → new); conflict → **Modified** with the word "conflict".
- Shown grouped by type. Each line shows the file name and its folder, relative to the project. Nothing is clickable.
- When the project is a subfolder of a bigger repo, git is run with `-- .` and the prefix from `git rev-parse --show-prefix` is stripped. A rename from outside the folder shows the outside path as `../…`.
- Refreshed when the project is selected, when the tab is opened, when a session in that project goes Idle, and with a small ↻ button. No polling and no file watcher.
- Capped at 500 lines, then "and N more".
- Not a git project → "Not a git project". Git not installed → "Git isn't installed".

## 7. Saved data

`%LOCALAPPDATA%\ClaudeDeck\state.json`, written atomically (temp file, then rename):

- `projects`: id, folder path, display name, order
- `sessions`: deck id, latest Claude conversation id (§4.1; Plan 2 adds this field next to the id), project id, name, `renamed` flag, created, last active. Only sessions shown in the sidebar are stored; closing a session removes it (its conversation stays in Claude's history, §4.6)
- `settings`: auto-sleep minutes, claude path, window size and position

No secrets are stored. Conversations stay in Claude's own files.

## 8. Error handling

| Problem | What the user sees |
|---|---|
| `claude.exe` not found | v1: the spawn error is shown in the footer. The setting is in Settings. A dedicated setup message is deferred to v1.1 |
| Wake fails (conversation file missing or unreadable) | The session shows "Can't resume" with **Start a new session here** |
| Project folder moved or deleted | The project shows "Folder missing" with **Remove**. **Locate…** is deferred to v1.1: Claude keys conversations by folder, so after a move `--resume` fails and the fallback retry would quietly start an empty conversation |
| Helper crashes | The window shows "Reconnecting…", restarts the helper, and every session comes back asleep |
| Hooks never report for a session | The session stays in the safe default (§4.2) and never auto-sleeps |
| A Claude process exits by itself | The session becomes Asleep |
| Pipe connection refused (another user, bad ACL) | The window refuses to start and says why |
| Git errors | Shown in the Changes tab as one line |

## 9. Install

- `pnpm tauri build` produces a per-user installer (NSIS, no admin). `deck-host.exe` ships next to the window exe.
- Start menu shortcut. Nothing starts at login: the window starts the helper.
- Uninstall removes the program. It leaves `%LOCALAPPDATA%\ClaudeDeck` unless the user ticks "remove my settings".

## 10. Testing

- **Spike first, with the real Claude 2.1.303**, before the main build. It confirms:
  - `--session-id` and `--resume` keep the same ID.
  - Hooks passed with `--settings` fire for every event in §4.2, and `CLAUDE_DECK_SESSION` reaches the hook.
  - Which `Notification` types mean "permission or question" and which mean "waiting for input".
  - Whether `Stop` fires after Esc.
  - Ctrl+C twice exits cleanly.
  - A `vt100` snapshot redraws Claude's screen correctly in xterm.js. This one needs a terminal host, so it is the first task of Plan 2 (the helper), with raw output replay as the fallback.

  Anything that turns out different gets fixed in this spec before the code that depends on it.
- **Unit tests (Rust, deck-core):** every state transition and sleep rule, including the safe default; the `git status` parser for each status code and for renames with `-z`; folder-name encoding against the real names in `~/.claude/projects`; saved-state load and save, including a corrupt file.
- **Integration tests (deck-host + tests/fake-claude):** fake-claude prints a prompt, runs the hook command for scripted events, and exits on Ctrl+C. These cover new → working → idle → auto-sleep → wake; Needs you never sleeps; window close with "keep running"; reattach snapshot; helper restart with everything asleep. No Claude tokens are used.
- **UI tests (Vitest + Vue Test Utils):** sidebar dots and badges per state, Changes grouping and the 500 cap, the close dialog choices, the context menus.
- **Manual check on this VM** with real Claude: 10 sessions over 4 projects, measure memory before and after auto-sleep.

## 11. Out of scope for v1

- A built-in file browser, editor, diff view or opening files from the Changes tab
- Windows toast notifications (the taskbar flash and window title in §5 cover "Needs you"; nothing is shown when a session simply finishes)
- Several windows, or several helpers per user
- Claude in the cloud, or a custom Claude client to share one runtime
- macOS and Linux
