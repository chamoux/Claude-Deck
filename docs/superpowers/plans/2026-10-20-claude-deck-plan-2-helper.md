# Claude Deck Plan 2: The Helper (`deck-host`) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax. Tasks in the same **wave** touch different files and can run **in parallel** (the user asked for parallel subagents on 2026-10-20).

**Goal:** Build `deck-host.exe`, the background helper that owns every Claude session and every project shell. It runs them in ConPTY terminals, keeps a headless screen per terminal so a window can reattach, applies Claude's hook events to the session states, auto-sleeps Idle sessions, and serves a window over a current-user-only named pipe. Everything is testable without the window.

**Architecture:**
- `deck-host` is one crate with a library (all logic) and two binaries: `deck-host` (subcommands `serve` and `hook`) and `fake-claude` (tests only).
- The heart of the library is `host::core`, a pure, synchronous state machine: `Core::handle(Input, now_ms) -> Vec<Effect>`. It never touches processes or pipes. `host::runtime` is the tokio shell around it. Runtime turns pipe frames, PTY output and timer ticks into `Input`s, and carries out the `Effect`s (spawn, write, kill, send, save). This keeps the hard logic unit-testable.
- The window protocol messages live in `deck-proto::messages`.

**Tech stack** (versions compile-verified on this VM, 2026-10-20):
- portable-pty 0.9.0
- vt100 0.18.1
- tokio 1.54
- sysinfo 0.39.6
- windows 0.62.2
- serde, serde_json, uuid, thiserror

**Spec:** `docs/superpowers/specs/2026-10-19-claude-deck-design.md`. Plan 1 left notes in `docs/superpowers/plans/2026-10-19-claude-deck-plan-1-core.md` ("Notes for Plan 2"). The spike results are in `spike/FINDINGS.md`.

---

## House rules (every task)

- **Never run `git commit` or `git push`.** Stage with `git add` only.
- Prefix cargo commands with `export PATH="$HOME/.cargo/bin:$PATH" && export CARGO_TARGET_DIR=/c/tmp/deck-target &&`. A single shared target dir means dependencies compile once, and the cargo lock serialises builds, which the VM's RAM needs. Use `-j 2`.
- **Correction (2026-10-20, from wave 1):** the shared target dir is NOT safe across workspace copies. Cargo gives `deck-host` the same metadata hash in every copy, so parallel agents overwrote each other's test binaries.
  - A private copy uses its own `CARGO_TARGET_DIR=/c/tmp/deck-target-<task>`, deleted at the end.
  - `/c/tmp/deck-target` is used only for checks in the real repo.
- **Changes agreed during wave 2:**
  - `Effect::MeasureMemory` carries `targets: Vec<Target>`, and the runtime maps them to pids.
  - The runtime ticks every **250 ms**, not every second, so the second Ctrl+C lands about 300 ms after the first.
  - `CoreConfig` gains `ctrl_c_gap_ms` (300) and `memory_every_ms` (5000).
- **Parallel safety:** a wave-1 or wave-2 agent works in its own copy, so its red (failing) steps never break another agent's build.
  ```bash
  rm -rf /c/tmp/ws-<task> && mkdir -p /c/tmp/ws-<task> && cp -r /c/Users/Chamoux/Desktop/claude-deck/{Cargo.toml,Cargo.lock,rust-toolchain.toml,crates} /c/tmp/ws-<task>/
  ```
  - Edit and test only the files your task owns.
  - When green, copy only those files back to the repo.
  - Then run `cargo test -p <crate> <filter>` once in the repo. If another agent's half-copied file breaks that build, wait a minute and retry.
  - `git add` your files, and retry if you hit `index.lock`.
  - Delete your copy.
- No new dependencies. Task 0 adds every dependency. If you need another, stop and report NEEDS_CONTEXT.
- Verified gotchas (research, 2026-10-20). Each one is handled by a named task:
  1. ConPTY's first output is `ESC[6n` (cursor position query). The host must answer `ESC[<row>;<col>R`, or the child dies with 0xC0000142. → Task 3 + Task 9
  2. A PTY reader does not reach EOF when the child exits. It ends only when the master is dropped. → Task 6
  3. `vt100` 0.18 takes `NonZeroU16` sizes, and `contents*()` returns `String`. → Task 2
  4. A process started from Git Bash inherits "ignore Ctrl+C". The helper calls `SetConsoleCtrlHandler(None, false)` at start, or `\x03` does nothing. → Task 10
  5. A named pipe needs `first_pipe_instance(true)` plus an SDDL `D:P(A;;GA;;;<SID>)` security descriptor. A second first instance fails with ERROR_ACCESS_DENIED. → Task 4
  6. Hook commands run through a POSIX shell, so paths in hooks.json use forward slashes and are quoted. → Task 7

## Waves

| Wave | Tasks (parallel within a wave) |
|---|---|
| 0 | Task 0: crate skeleton, all dependencies, module stubs, store additions for shells (done by the controller) |
| 1 | Task 2 screen · Task 3 dsr · Task 4 pipe · Task 5 messages · Task 6 pty · Task 8 git_runner + memory + locate · Task 11 fake-claude |
| 2 | Task 7 hook command · Task 9 host core · Task 1 real-Claude spike (needs 2, 3, 6) |
| 3 | Task 10 runtime + main |
| 4 | Task 12 integration tests · Task 13 whole-plan checks and final review |

---

## File structure

| File | Owner task | Responsibility |
|---|---|---|
| `crates/deck-host/Cargo.toml` | 0 | All dependencies, two `[[bin]]` entries |
| `crates/deck-host/src/lib.rs` | 0 | Module list |
| `crates/deck-host/src/screen.rs` | 2 | `Screen`: headless terminal (vt100) with resize and snapshot |
| `crates/deck-host/src/dsr.rs` | 3 | `DsrFilter`: removes `ESC[6n` from output and counts the queries, even when split across reads |
| `crates/deck-host/src/pipe.rs` | 4 | Current user SID, pipe name, current-user-only pipe server, client connect, single-instance mutex |
| `crates/deck-host/src/framed.rs` | 4 | Async read and write of `deck_proto::Frame`s on any `AsyncRead`/`AsyncWrite` |
| `crates/deck-proto/src/messages.rs` | 5 | `Request`, `Event`, `StateView`, `SessionView`, `ShellView`, `GitResult`, `Target`, `ClientKind` |
| `crates/deck-host/src/pty.rs` | 6 | `PtyProcess`: spawn in ConPTY, reader thread, waiter thread, write, resize, kill |
| `crates/deck-host/src/hook_cmd.rs` | 7 | hooks.json text and file, and `run_hook` (the `deck-host hook` subcommand) |
| `crates/deck-host/src/git_runner.rs` | 8 | Runs git for the Changes tab and returns `GitResult` |
| `crates/deck-host/src/memory.rs` | 8 | Private bytes of process trees (sysinfo) |
| `crates/deck-host/src/locate.rs` | 8 | Finds `claude.exe`, the shell exe, and the `%LOCALAPPDATA%\ClaudeDeck` paths |
| `crates/deck-host/src/host/mod.rs` | 0 | `pub mod core; pub mod runtime;` |
| `crates/deck-host/src/host/core.rs` | 9 | `Core`, `Input`, `Effect`: all decisions |
| `crates/deck-host/src/host/runtime.rs` | 10 | tokio actor: pipe server, PTY events, timers, effects |
| `crates/deck-host/src/main.rs` | 10 | `deck-host serve` / `deck-host hook` |
| `crates/deck-host/src/bin/fake-claude.rs` | 11 | Test stand-in for `claude.exe` |
| `crates/deck-host/tests/end_to_end.rs` | 12 | Integration tests through the real pipe with fake-claude |
| `spike/pty-claude/` | 1 | Spike program run against real Claude, plus `spike/FINDINGS-2.md` |
| `crates/deck-core/src/store.rs` | 0 | Adds `ShellKind`, `Settings.shell`, and per-project panel state |

---

### Task 0 (wave 0, controller): skeleton and store additions

- [ ] **Step 1: Write `crates/deck-host/Cargo.toml`**

```toml
[package]
name = "deck-host"
version.workspace = true
edition.workspace = true
publish.workspace = true

[lib]
path = "src/lib.rs"

[[bin]]
name = "deck-host"
path = "src/main.rs"

[[bin]]
name = "fake-claude"
path = "src/bin/fake-claude.rs"

[dependencies]
deck-core.workspace = true
deck-proto.workspace = true
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true
thiserror.workspace = true
portable-pty = "0.9.0"
vt100 = "0.18.1"
tokio = { version = "1.54", features = ["rt-multi-thread", "macros", "net", "io-util", "time", "sync"] }
sysinfo = { version = "0.39.6", default-features = false, features = ["system"] }

[dependencies.windows]
version = "0.62.2"
features = [
  "Win32_Foundation", "Win32_Security", "Win32_Security_Authorization",
  "Win32_System_Threading", "Win32_System_Console",
]

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: Update the workspace and the proto crate**
  - Add `"crates/deck-host"` to `members` in the root `Cargo.toml`.
  - In `crates/deck-proto/Cargo.toml`, add `deck-core.workspace = true`, `serde.workspace = true` and `serde_json.workspace = true`.
  - In `crates/deck-proto/src/lib.rs`, add `pub mod messages;` and create `messages.rs` containing only `//! Messages between the window and the helper (spec §3.4).`

- [ ] **Step 3: Module stubs.** `crates/deck-host/src/lib.rs`:
```rust
//! Claude Deck's helper: owns Claude sessions and project shells (spec §3.2).

pub mod dsr;
pub mod framed;
pub mod git_runner;
pub mod hook_cmd;
pub mod host;
pub mod locate;
pub mod memory;
pub mod pipe;
pub mod pty;
pub mod screen;
```
Make each module file a one-line `//!` doc comment.
- `host/mod.rs` holds `pub mod core; pub mod runtime;`, and both of those files get a doc line.
- `main.rs` holds `fn main() {}`.
- `src/bin/fake-claude.rs` holds `fn main() {}`.

- [ ] **Step 4: Store additions for the project terminal** (spec §5), test-first in `crates/deck-core/src/store.rs`:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShellKind { #[default] PowerShell, GitBash, Cmd }

// in Settings (serde default):
pub shell: ShellKind,

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellPanel { pub open: bool, pub width: u32 }
impl Default for ShellPanel { fn default() -> Self { Self { open: false, width: 480 } } }

// in Project (serde default):
#[serde(default)]
pub shell_panel: ShellPanel,

impl SavedState {
    pub fn project(&self, project_id: &str) -> Option<&Project>;
    pub fn set_shell_panel(&mut self, project_id: &str, panel: ShellPanel); // width clamped to 240..=1600
}
```
Tests:
- an old file without these fields loads with PowerShell, closed, width 480
- the settings round trip keeps `git_bash`
- `set_shell_panel` clamps 100 to 240 and 9999 to 1600
- `project()` finds a project by id

- [ ] **Step 5: Build once** (this also warms the shared target)

`cargo build --workspace -j 2 && cargo test --workspace -j 2`. Expected: everything compiles, and all deck-core tests pass, including the new ones.

- [ ] **Step 6: Stage** `git add Cargo.toml Cargo.lock crates/`

---

### Task 1 (wave 2): Spike, real Claude in our own PTY

**Owns:** `spike/pty-claude/` (its own tiny Cargo package, *not* a workspace member: give it its own `[workspace]` table) and `spike/FINDINGS-2.md`.

The spike copies `screen.rs`, `dsr.rs` and `pty.rs` from the repo once they are green, using `#[path = "../../../crates/deck-host/src/pty.rs"] mod pty;` and so on, so it exercises the real code. It drives real `claude` with `--settings` pointing at `spike/hooks.json` (forward slashes), in `C:\tmp\deck-spike2`, with `--permission-mode default`, at 120×40. It costs a few cents.

It must answer and record in FINDINGS-2.md:
1. Does Claude switch to the alternate screen (`ESC[?1049h`)? How many output bytes per minute of normal use?
2. Is `ESC[6n` sent once at start, or repeatedly? Does answering from `Screen::cursor()` keep Claude alive?
3. A snapshot after a finished turn, fed into a fresh `Screen`, gives the same `contents()` as the original.
4. Ctrl+C: does writing `\x03`, waiting 300 ms, then `\x03` make Claude exit? Measure the exit time. What does a single `\x03` do?
5. With `idle_prompt`: finish a turn (prompt `reply with the word ok`), wait 75 s, and capture the Notification payload, if any.
6. `/clear`: type `/clear\r`. Does `SessionStart` arrive with `source: "clear"`, and with a new `session_id`? Is `SessionEnd` reason `"clear"`?
7. Esc on a permission prompt: ask `create a file x.txt containing 1`. When the Notification arrives, write `\x1b`. Which hooks follow?

If any answer contradicts Task 9's rules, update the spec §4.1/§4.2 tables and Task 9's tests before Task 9 is reviewed.

---

### Task 2 (wave 1): `screen.rs`

**Owns:** `crates/deck-host/src/screen.rs`.

```rust
use std::num::NonZeroU16;

pub const SCROLLBACK_LINES: usize = 5000;

/// Headless terminal for one PTY, so a window can reattach and redraw (spec §3.2).
pub struct Screen { parser: vt100::Parser }

impl Screen {
    /// Sizes below 1 are treated as 1.
    pub fn new(rows: u16, cols: u16) -> Self;
    pub fn process(&mut self, bytes: &[u8]);
    pub fn resize(&mut self, rows: u16, cols: u16);
    pub fn size(&self) -> (u16, u16);
    /// Zero-based (row, col) of the cursor.
    pub fn cursor(&self) -> (u16, u16);
    pub fn alternate_screen(&self) -> bool;
    /// Bytes that redraw scrollback, then the screen, then the cursor and modes, in a fresh terminal of the same size.
    pub fn snapshot(&mut self) -> Vec<u8>;
    /// Plain text of the visible screen (tests and diagnostics).
    pub fn contents(&self) -> String;
}
```

Snapshot algorithm, compile-verified on this VM:
```rust
fn snapshot(p: &mut vt100::Parser) -> Vec<u8> {
    let (rows, cols) = { let (r, c) = p.screen().size(); (r.get(), c.get()) };
    let sb = p.screen().scrollback_len();
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b[?25l\x1b[m\x1b[2J\x1b[H");
    for i in 0..sb {                                  // oldest first
        p.screen_mut().set_scrollback(sb - i);        // top visible row == scrollback row i
        let row = p.screen().rows_formatted(0, cols).next().unwrap();
        out.extend_from_slice(row.as_bytes());
        out.extend_from_slice(b"\x1b[m\r\n");
    }
    if sb > 0 { out.extend(std::iter::repeat(b'\n').take(rows as usize - 1)); }
    p.screen_mut().set_scrollback(0);
    out.extend_from_slice(b"\x1b[m\x1b[2J\x1b[H");
    out.extend_from_slice(p.screen().state_formatted().as_bytes());
    out
}
```
Alternate screen: `state_formatted()` does not emit `ESC[?1049h`. When `alternate_screen()` is true, emit the main-screen snapshot (no scrollback is readable then; send just `\x1b[2J\x1b[H`), then `\x1b[?1049h`, then the alternate screen's `state_formatted()`. Write that as its own branch.

Tests (write them first):
- `round_trip_reproduces_screen_scrollback_and_cursor`:
  - Build a 10×40 screen. Feed it 25 coloured numbered lines, bold/underline/RGB text, `ESC[3;5H` and some text.
  - Feed `snapshot()` into a new 10×40 `Screen`.
  - Expect equal `contents()`, equal `cursor()`, equal scrollback length (read through `parser.screen().scrollback_len()` behind a `#[cfg(test)]` accessor), and equal text in each scrollback row.
- `snapshot_restores_the_live_view`: after `snapshot()`, `contents()` is the same as before (the scrollback viewport is reset to 0).
- `resize_changes_size` and `zero_sizes_become_one`.
- `alternate_screen_snapshot_enters_the_alternate_screen`: feed `\x1b[?1049h` and text. The new screen reports `alternate_screen() == true` and the same `contents()`.

---

### Task 3 (wave 1): `dsr.rs`

**Owns:** `crates/deck-host/src/dsr.rs`.

```rust
/// Removes cursor-position queries (`ESC [ 6 n`) from PTY output and counts them,
/// so the host can answer them itself (ConPTY waits for the answer) and the
/// window's terminal does not answer a second time.
#[derive(Debug, Default)]
pub struct DsrFilter { carry: Vec<u8> }

impl DsrFilter {
    /// Returns the bytes to pass on and how many queries were removed. A partial
    /// `ESC`, `ESC [` or `ESC [ 6` at the end is held back until the next call.
    pub fn feed(&mut self, bytes: &[u8]) -> (Vec<u8>, usize);
}

/// The answer to one query: `ESC [ row ; col R`, 1-based.
pub fn cursor_report(row0: u16, col0: u16) -> Vec<u8>;
```
Tests:
- the plain text passes through
- one query is removed and counted
- two queries in one chunk
- a query split as `\x1b` | `[6n`, and as `\x1b[` | `6n`
- `\x1b[6m` (not a query) passes through unchanged, including when split after `\x1b[6`
- a trailing lone `\x1b` is held back, then released when the next chunk is not part of a query
- `cursor_report(0,0) == b"\x1b[1;1R"`
- `cursor_report(4,9) == b"\x1b[5;10R"`

---

### Task 4 (wave 1): `pipe.rs` + `framed.rs`

**Owns:** `crates/deck-host/src/pipe.rs`, `crates/deck-host/src/framed.rs`.

`pipe.rs` API (core code verified on this VM, 2026-10-20):
```rust
pub fn current_user_sid() -> std::io::Result<String>;           // OpenProcessToken + GetTokenInformation(TokenUser) + ConvertSidToStringSidW
pub fn pipe_name(sid: &str) -> String;                           // format!(r"\\.\pipe\claude-deck-{sid}")
/// Owns a security descriptor that allows only one SID (SDDL `D:P(A;;GA;;;<SID>)`).
/// Freed with LocalFree on drop. Keep it alive while pipe instances are created.
pub struct PipeSecurity { /* PSECURITY_DESCRIPTOR + SECURITY_ATTRIBUTES */ }
impl PipeSecurity { pub fn only(sid: &str) -> std::io::Result<Self>; }
// Safety: the descriptor is read-only after creation.
unsafe impl Send for PipeSecurity {} unsafe impl Sync for PipeSecurity {}
/// `first`: use first_pipe_instance(true) for the very first instance (anti-squatting).
/// Always reject_remote_clients(true).
pub fn create_server(name: &str, sec: &PipeSecurity, first: bool)
    -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer>;
/// Retries ERROR_PIPE_BUSY (231) every 50 ms, up to `wait`.
pub async fn connect_client(name: &str, wait: std::time::Duration)
    -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient>;
/// Named mutex `Local\claude-deck-<sid>`. Ok(None) when another helper holds it.
pub struct SingleInstance(/* HANDLE */);
impl SingleInstance { pub fn acquire(sid: &str) -> std::io::Result<Option<Self>>; }
```
The verified snippet (OpenProcessToken → TOKEN_USER → ConvertSidToStringSidW → ConvertStringSecurityDescriptorToSecurityDescriptorW → SECURITY_ATTRIBUTES → `ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create_with_security_attributes_raw(&name, &mut sa as *mut _ as *mut c_void)`) is in the research report and in `C:\tmp\api-check\src\bin\pipe.rs`. The mutex snippet is in `C:\tmp\api-check\src\bin\mem.rs`. Write the pipe name literally as `r"\\.\pipe\..."`.

`framed.rs` API:
```rust
use deck_proto::{Frame, FrameDecoder};
/// Reads until one complete frame. Ok(None) on a clean EOF between frames.
/// A decode error becomes io::ErrorKind::InvalidData.
pub async fn read_frame<R: tokio::io::AsyncRead + Unpin>(r: &mut R, dec: &mut FrameDecoder) -> std::io::Result<Option<Frame>>;
pub async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, frame: &Frame) -> std::io::Result<()>;
```
Tests:
- pipe: `pipe_name` format; `current_user_sid` starts with `S-1-5-`.
- Under `#[tokio::test]`, a server with a test-unique name accepts a client, which writes a framed `Control(b"ping")` and gets `Control(b"pong")` back.
- A second `create_server(..., first: true)` on the same name fails while the first is alive.
- `SingleInstance::acquire` twice gives Some, then None (use a test-unique sid string).
- framed: `tokio::io::duplex` round-trips three frames; EOF gives `Ok(None)`; garbage gives `InvalidData`.

---

### Task 5 (wave 1): `deck-proto::messages`

**Owns:** `crates/deck-proto/src/messages.rs`.

Every message travels as JSON inside `Frame::Control`. Terminal bytes travel as `Frame::Output`/`Frame::Input`, keyed by a u32 **index** that the helper assigns when a terminal is attached.

```rust
use deck_core::{claude_dir::PastSession, git::ChangeList, state::SessionState, store::{Project, Settings, ShellPanel}};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind { Window, Hook }

/// A terminal the window can attach: a Claude session (by deck id) or a project shell.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target { Session { deck_id: String }, Shell { project_id: String } }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello { client: ClientKind },
    GetState,
    AddProject { path: String },
    RemoveProject { project_id: String },
    NewSession { project_id: String },
    Wake { deck_id: String },
    /// `force` = the user confirmed sleeping a non-Idle session.
    Sleep { deck_id: String, force: bool },
    CloseSession { deck_id: String, force: bool },
    Rename { deck_id: String, name: String },
    /// Starts the shell if needed; for a session the session must be awake.
    Attach { target: Target, rows: u16, cols: u16 },
    Detach { target: Target },
    Resize { target: Target, rows: u16, cols: u16 },
    CloseShell { project_id: String },
    SetShellPanel { project_id: String, panel: ShellPanel },
    GitStatus { project_id: String },
    ListPastSessions { project_id: String },
    OpenPastSession { project_id: String, conversation_id: String },
    SetSettings { settings: Settings },
    /// The window is closing. Idle sessions always sleep; `keep_running == false`
    /// also sleeps busy sessions and closes shells (spec §4.4, §5).
    WindowClosing { keep_running: bool },
    /// From `deck-host hook`: Claude's raw hook JSON for the session with this deck id.
    HookEvent { deck_id: String, payload: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionView {
    pub deck_id: String,
    pub project_id: String,
    pub name: Option<String>,
    pub state: SessionState,
    pub background_work: bool,
    /// Set when the last wake failed, e.g. "Can't resume" (spec §8).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShellView { pub project_id: String, pub running: bool }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateView {
    pub projects: Vec<Project>,
    pub sessions: Vec<SessionView>,
    pub shells: Vec<ShellView>,
    pub settings: Settings,
    /// Projects whose folder no longer exists (spec §8).
    pub missing_folders: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GitResult { Ok { list: ChangeList }, NotGit, GitMissing, Error { message: String } }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    State { state: StateView },
    SessionChanged { session: SessionView },
    ShellChanged { shell: ShellView },
    /// The next Frame::Output for `index` is a full redraw (snapshot); reset the terminal first.
    Attached { target: Target, index: u32 },
    Detached { target: Target },
    Memory { awake: u32, bytes: u64 },
    GitStatus { project_id: String, result: GitResult },
    PastSessions { project_id: String, sessions: Vec<PastSession> },
    Error { message: String },
}

pub fn encode_request(r: &Request) -> crate::Frame;      // Frame::Control(serde_json::to_vec)
pub fn encode_event(e: &Event) -> crate::Frame;
pub fn decode_request(json: &[u8]) -> Result<Request, serde_json::Error>;
pub fn decode_event(json: &[u8]) -> Result<Event, serde_json::Error>;
```
Note: `Change`, `ChangeList`, `PastSession`, `Project`, `Settings` and `ShellPanel` must derive both `Serialize` and `Deserialize`. Some derive only `Serialize` today; add `Deserialize` in deck-core (the only allowed deck-core edit in this task).

Tests:
- Exact JSON for three messages, to pin the wire format the TypeScript UI will use:
  - `Request::Attach{target: Target::Shell{project_id:"p"}, rows:40, cols:120}` → `{"type":"attach","target":{"kind":"shell","project_id":"p"},"rows":40,"cols":120}`
  - `Event::Memory{awake:2,bytes:5}` → `{"type":"memory","awake":2,"bytes":5}`
  - `GitResult::NotGit` → `{"status":"not_git"}`
- A round trip of every `Request` variant and of an `Event::State` holding one project, session and shell.

---

### Task 6 (wave 1): `pty.rs`

**Owns:** `crates/deck-host/src/pty.rs`.

```rust
pub struct SpawnSpec {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
    pub cwd: std::path::PathBuf,
    pub env: Vec<(String, String)>,   // added to the inherited environment
    pub rows: u16,
    pub cols: u16,
}

pub enum PtyEvent { Output(Vec<u8>), Exited(Option<u32>) }

/// One child in a ConPTY. Events arrive on the callback from background threads,
/// in order. `Exited` is always the last event.
pub struct PtyProcess { /* master: Mutex<Option<Box<dyn MasterPty + Send>>>, writer: Mutex<Box<dyn Write + Send>>, killer: Mutex<Box<dyn ChildKiller + Send + Sync>>, pid: Option<u32> */ }

impl PtyProcess {
    pub fn spawn(spec: &SpawnSpec, on_event: impl FnMut(PtyEvent) + Send + 'static) -> std::io::Result<std::sync::Arc<Self>>;
    pub fn write(&self, bytes: &[u8]) -> std::io::Result<()>;   // flushes
    pub fn resize(&self, rows: u16, cols: u16) -> std::io::Result<()>;
    pub fn kill(&self) -> std::io::Result<()>;
    pub fn pid(&self) -> Option<u32>;
}
```
Behaviour:
- Spawn with `native_pty_system().openpty`, then `CommandBuilder`, `spawn_command`, and **drop the slave right after the spawn**.
- **Reader thread:** reads 16 KB chunks and sends `Output`.
- **Waiter thread:** owns the `Child` and calls `wait()`. Kill goes through `child.clone_killer()`. If portable-pty 0.9 lacks it, poll `try_wait` every 100 ms instead and say so.
- **Exit order:** on exit, the waiter first drops the master (`take()` from the Mutex), which makes the reader hit EOF. Exactly one `Exited`, carrying the exit code, is sent after the reader thread has finished. Use a channel or join, so no `Output` can follow `Exited`.
- **Cursor queries are not answered here.** That is the core's job, through `dsr` + `screen`.

Tests: use `cmd.exe /c echo hello`, and answer the first `ESC[6n` from the callback by writing `\x1b[1;1R`. The callback captures an `Arc<Mutex<Option<Arc<PtyProcess>>>>` that is set after spawn.
- The output contains `hello`, then `Exited(Some(0))` arrives, with no events after it.
- `kill()` on `cmd.exe /k` (stays open) leads to `Exited(_)` within 5 s.
- `resize(30,100)` returns Ok.

---

### Task 7 (wave 2): `hook_cmd.rs`

**Owns:** `crates/deck-host/src/hook_cmd.rs`. **Needs:** Tasks 4 and 5.

```rust
/// hooks.json for `--settings`: SessionStart, UserPromptSubmit, PreToolUse and PostToolUse
/// (matcher "*"), Notification, Stop, SessionEnd, each running `"<exe>" hook`.
/// The exe path uses forward slashes inside quotes (Claude runs hooks through a POSIX shell).
/// Never sets "async".
pub fn hooks_json(host_exe: &std::path::Path) -> String;
pub fn write_hooks_file(path: &std::path::Path, host_exe: &std::path::Path) -> std::io::Result<()>;
/// The `deck-host hook` subcommand. Reads all of stdin. If CLAUDE_DECK_SESSION is unset or
/// empty, returns at once. Otherwise it connects (wait 2 s), sends Hello{Hook} and then
/// HookEvent{deck_id, payload}, and waits up to 2 s for one byte (deck_proto::HOOK_ACK).
/// Every failure is silent. The caller always exits 0.
pub async fn run_hook(stdin: String, deck_id: Option<String>, pipe_name: &str);
```
Tests:
- `hooks_json(Path::new(r"C:\Program Files\Claude Deck\deck-host.exe"))` parses as JSON.
- Every command equals `"C:/Program Files/Claude Deck/deck-host.exe" hook`.
- The 7 event keys are present.
- No `async` key appears anywhere.
- `run_hook` against a test pipe server (from Task 4) with a unique name:
  - the server receives Hello{Hook}, then HookEvent with the right deck id and payload;
  - the server replies with `HOOK_ACK`, and `run_hook` returns well under 2 s;
  - with `deck_id: None` it returns at once and sends nothing;
  - with no server at all it returns within about 2.5 s.

---

### Task 8 (wave 1): `git_runner.rs`, `memory.rs`, `locate.rs`

**Owns** those three files.

```rust
// git_runner.rs
/// Runs `git rev-parse --show-prefix`, then `git status --porcelain=v1 -z --untracked-files=all -- .`
/// in `project`, with CREATE_NO_WINDOW (0x08000000) so no console flashes.
/// Results go through deck_core::git::{parse_porcelain_z, strip_prefix, summarize(.., DEFAULT_CAP)}.
/// A "not a git repository" failure → NotGit, git not found → GitMissing, anything else → Error(first stderr line).
pub fn git_status(project: &std::path::Path) -> deck_proto::messages::GitResult;

// memory.rs
pub struct MemoryMeter { /* sysinfo::System, reused between calls */ }
impl MemoryMeter {
    pub fn new() -> Self;
    /// Refreshes once, then sums Process::virtual_memory() (= PrivateUsage on Windows)
    /// over each root pid and all its descendants. Uses a seen-set against pid reuse.
    pub fn private_bytes(&mut self, roots: &[u32]) -> u64;
}

// locate.rs
/// %LOCALAPPDATA%\ClaudeDeck (created if missing).
pub fn app_dir() -> std::io::Result<std::path::PathBuf>;
pub fn state_path(app_dir: &std::path::Path) -> std::path::PathBuf;   // state.json
pub fn hooks_path(app_dir: &std::path::Path) -> std::path::PathBuf;   // hooks.json
/// The setting if it names an existing file, else claude.exe on PATH, else %USERPROFILE%\.local\bin\claude.exe.
pub fn find_claude(setting: Option<&str>) -> Option<std::path::PathBuf>;
/// PowerShell: pwsh.exe on PATH, else powershell.exe. Git Bash: C:\Program Files\Git\bin\bash.exe
/// (args ["--login","-i"]). Cmd: cmd.exe. Returns (program, args).
pub fn shell_command(kind: deck_core::store::ShellKind) -> (std::path::PathBuf, Vec<String>);
```
Tests:
- git: run in a tempdir with `git init`, write a file, commit one file, modify it, add an untracked file and a subfolder project. Expect Modified + Created at the root. The subfolder sees only its own files, with stripped paths. A non-git tempdir → NotGit.
- memory: `private_bytes(&[std::process::id()]) > 1_000_000`, and an unknown pid gives 0.
- locate: `find_claude(Some(<a temp file path>))` returns it; `find_claude(Some(r"C:\nope.exe"))` falls back to search without panicking. `shell_command(Cmd).0` ends with `cmd.exe`.

---

### Task 9 (wave 2): `host/core.rs`, every decision

**Owns:** `crates/deck-host/src/host/core.rs`. **Needs:** Tasks 2, 3 and 5, plus deck-core. Use the standard model (judgment task).

```rust
pub type ClientId = u64;

pub enum Input {
    WindowConnected(ClientId),
    ClientGone(ClientId),
    Request(ClientId, deck_proto::messages::Request),
    TerminalInput { index: u32, bytes: Vec<u8> },
    PtyOutput { target: Target, bytes: Vec<u8> },
    PtyExited { target: Target },
    SpawnFailed { target: Target, error: String },
    Tick,                              // every second
    GitDone { client: ClientId, project_id: String, result: GitResult },
    PastDone { client: ClientId, project_id: String, sessions: Vec<PastSession> },
    MemoryMeasured { awake: u32, bytes: u64 },
}

pub enum Effect {
    Spawn { target: Target, spec: crate::pty::SpawnSpec },
    Write { target: Target, bytes: Vec<u8> },
    Resize { target: Target, rows: u16, cols: u16 },
    Kill { target: Target },
    Send { client: ClientId, frame: deck_proto::Frame },
    Broadcast { frame: deck_proto::Frame },            // to every window client
    HookAck { client: ClientId },                      // write HOOK_ACK and close
    RunGit { client: ClientId, project_id: String, path: std::path::PathBuf },
    ListPast { client: ClientId, project_id: String, path: std::path::PathBuf, exclude: std::collections::HashSet<String> },
    MeasureMemory { pids: Vec<u32> },                  // runtime fills pids; core sends the targets
    Save { state: deck_core::store::SavedState },      // runtime writes it on a blocking thread
    Exit,
}

pub struct CoreConfig {
    pub claude_exe: std::path::PathBuf,
    pub hooks_file: std::path::PathBuf,
    pub shell: (std::path::PathBuf, Vec<String>),
    pub sleep_grace_ms: i64,     // 5000: wait after the Ctrl+C pair, then Kill
    pub idle_exit_ms: i64,       // 10000: no window and nothing awake → Exit
}

pub struct Core { /* SavedState, live terminals, windows, attachments, timers */ }
impl Core {
    pub fn new(state: deck_core::store::SavedState, config: CoreConfig) -> Self;
    pub fn handle(&mut self, input: Input, now_ms: i64) -> Vec<Effect>;
}
```

Rules (each one needs at least one unit test, driven only through `handle`):
1. **Spawn arguments.**
   - New session: `--session-id <deck id> --settings <hooks_file>`.
   - Wake: `--resume <conversation_id> --settings <hooks_file>`.
   - Env: `CLAUDE_DECK_SESSION=<deck id>`, `TERM=xterm-256color`, `COLORTERM=truecolor`.
   - cwd: the project path. A missing folder → no spawn, `Event::Error` plus `missing_folders`.
   - The terminal starts at 120×40 until the first Attach or Resize. Shells use `config.shell` in the project folder.
2. **Status.** A spawned session gets `SessionStatus::started()`. `PtyExited` → `on_exit()`, the live terminal is removed, and `SessionChanged` is broadcast. The session stays in the saved sidebar (Asleep).
3. **Hooks.**
   - `HookEvent` → `deck_core::hook::parse_hook(payload)` → `status.apply(event, now)`.
   - On `SessionStart` (any source): `set_conversation(deck_id, call.conversation_id)`.
   - On `UserPromptSubmit`: `name_from_prompt`.
   - Always: `touch`, then `Save` and `SessionChanged` (only if something changed), then **`HookAck` last**.
   - Unknown deck id, or bad JSON → only `HookAck`.
4. **Output path** (`PtyOutput`):
   - First `dsr.feed`. For each query, write `cursor_report(screen.cursor())` back to that terminal.
   - Then `screen.process(clean)`.
   - Then `Send` `Frame::Output{index}` to each window attached to that target.
5. **Attach.**
   - Assign or keep an index per target.
   - Reply `Attached{target,index}`, then `Frame::Output{index, screen.snapshot()}`.
   - Apply the size: `Resize` the PTY and resize the Screen.
   - Attaching a session that is not awake → `Event::Error`.
   - Attaching a shell that is not running → `Spawn` it first. The snapshot of an empty screen is fine.
6. **Input.**
   - `TerminalInput{index}` → `Write`.
   - For a session: `status.on_input(now)`; if that changed the state, broadcast `SessionChanged`.
7. **Auto-sleep** (on `Tick`): every session where `should_auto_sleep(now, settings.auto_sleep_minutes)` → sleep.
8. **Sleep:**
   - `Write "\x03"`, then 300 ms later (on a `Tick` at or after that time) `Write "\x03"`.
   - After `sleep_grace_ms` with no `PtyExited` → `Kill`.
   - `Sleep{force:false}` on a session where `!can_sleep_without_asking()` → `Event::Error("Claude may be working")` and no action. `force:true` sleeps.
9. **Close session:** sleep it (force rules as in rule 8), then remove it from the saved state (`remove_session`) and broadcast `State`.
10. **WindowClosing{keep_running}:**
    - Idle sessions always sleep.
    - With `keep_running == false`: every session sleeps and every shell gets `Kill`.
    - Shells are also killed on `ClientGone` of the last window when `keep_running` was not chosen.
11. **Idle exit.** When no window is connected and nothing is awake (sessions or shells) for `idle_exit_ms` → `Exit`.
12. **Memory.** While a window is connected, every 5 s → `MeasureMemory{pids}`; `MemoryMeasured` → broadcast `Event::Memory`.
13. **GetState / Hello{Window}:** reply `Event::State`. Every saved-state change → `Save` (the runtime debounces).
14. **OpenPastSession:** `add_past_session`, then wake it.
15. **ListPastSessions:** `ListPast` with `exclude = conversation_ids()`. `PastDone` → `Send` `Event::PastSessions`.
16. **SetSettings** → `set_settings`. **SetShellPanel** → `set_shell_panel`. Each → `Save` + `State`.

Test helpers: build a `Core` with a temp project folder, call `handle`, and pattern-match on the effects. Feed `PtyOutput` containing `\x1b[6n` and assert a `Write` with `\x1b[1;1R`.

---

### Task 10 (wave 3): `host/runtime.rs` + `main.rs`

**Owns** those two files. **Needs:** everything above.

- `main.rs`:
  - `deck-host hook` → read stdin and call `run_hook` on a current-thread runtime, then exit 0.
  - `deck-host serve` (also the default) → run the multi-thread runtime.
  - First `SetConsoleCtrlHandler(None, false)`.
  - `SingleInstance::acquire`; if `None`, exit 0 (one helper per user).
  - `locate::app_dir`, `store::load`. On `Err`, log it and exit 1, so defaults are **never** saved over the file.
  - `write_hooks_file(hooks_path, current_exe)`, build `CoreConfig`, call `runtime::run`.
- `runtime::run(core, pipe_name, sec)`:
  - **One actor task** owns `Core` and receives `Input`s from an unbounded mpsc channel.
  - **Pipe server loop:** create the first instance with `first: true`, then repeat:
    - `connect().await`;
    - create the next instance;
    - spawn a client task that reads frames.
  - **Client tasks:**
    - The first frame decides the kind. `Hello{Hook}` means a hook client: its next frame is `HookEvent`, sent to the actor as an `Input::Request`. On `Effect::HookAck`, write the single `HOOK_ACK` byte and close.
    - A window client gets an outbound `mpsc` of `Frame`s and an `Input::WindowConnected` / `Input::ClientGone`.
  - **PTY callbacks** send `PtyOutput`/`PtyExited` into the actor channel. A 1-second `interval` sends `Tick`.
  - **Effects:**
    - `Spawn` → `PtyProcess::spawn` (`SpawnFailed` on error).
    - `RunGit`, `ListPast`, `MeasureMemory`, `Save` → `spawn_blocking`, posting the result back as an `Input`.
    - `Save` is debounced to at most one write per second, and the last state wins.
    - `Exit` → flush pending saves, then end the process.
- Logging: `eprintln!` lines to `%LOCALAPPDATA%\ClaudeDeck\logs\host-<date>.log`. Delete files older than 7 days at start.
- Tests here are covered by Task 12.

---

### Task 11 (wave 1): `fake-claude`

**Owns:** `crates/deck-host/src/bin/fake-claude.rs`.

It is a stand-in for `claude.exe` in tests. It drives hooks the real way: it reads its own `--settings <file>` and runs each hook command from that file through `bash -c`, exactly as Claude does, with the JSON on stdin. It does not call `deck_host::hook_cmd` directly, because that would bypass what is being tested.

Behaviour:
- On start it prints `fake-claude ready\r\n`, then `ESC[6n` once, and expects a cursor report back (reads until `R`).
- It runs the SessionStart hook with `source: "startup"`, or `"resume"` if `--resume <id>` was given. `session_id` = the `--session-id` or `--resume` value.
- It reads stdin lines (cooked console mode). Real Claude uses raw mode, where Ctrl+C is a key. In cooked mode, `\x03` written to the PTY arrives as a CTRL_C_EVENT, so it installs `SetConsoleCtrlHandler` with a handler that counts them. It first calls `SetConsoleCtrlHandler(None, false)`. Two within 2 s → it runs SessionEnd (`reason: "prompt_input_exit"`) and exits 0.
- A line `work <n>` → runs UserPromptSubmit (`prompt` = the line), waits n×100 ms printing dots, then runs Stop with empty `background_tasks`.
- A line `background` → UserPromptSubmit, then Stop with `background_tasks:[{"id":"t1","status":"running"}]`.
- A line `ask` → UserPromptSubmit, PreToolUse, then Notification `{"notification_type":"permission_prompt","message":"Claude needs your permission"}`. The next line `y` → PostToolUse, then Stop.
- A line `clear` → SessionEnd `reason:"clear"`, then SessionStart `source:"clear"` with a new `session_id`.

Test: it builds. Task 12 exercises it.

---

### Task 12 (wave 4): `tests/end_to_end.rs`

**Owns** that file. It needs everything above.

Each test:
- starts `Core` + `runtime::run` in-process, on a unique pipe name and a temp app dir;
- uses `CoreConfig.claude_exe = env!("CARGO_BIN_EXE_fake-claude")` and `hooks_file` = a real hooks.json that points at `env!("CARGO_BIN_EXE_deck-host")`;
- uses short timers (`sleep_grace_ms: 2000`, auto-sleep set to `Some(5)` minutes, driven by injecting `Input::Tick` with a fake clock through a `#[cfg(test)]` hook in runtime, or by setting minutes with a test-only constructor);
- talks to the helper as a window over the real pipe.

Scenarios:
1. new session → Idle after SessionStart → `work 3` → Working → Idle
2. the auto-sleep clock elapses → the session receives `\x03\x03` → exits → Asleep
3. wake → `--resume` with the same conversation id
4. `background` → stays Working, and auto-sleep never fires
5. `ask` → Needs you → typing `y` → Working → Idle
6. `clear` → the conversation id changes, and the next wake uses the new one
7. attach → a snapshot arrives, then live output; detach stops the output
8. a project shell (`cmd.exe`) → attach → type `echo hi\r` → output contains `hi`; `CloseShell` → `ShellChanged{running:false}`
9. `WindowClosing{keep_running:false}` → everything stops; with `true`, a Working session survives
10. a second helper instance exits at once (single instance)

---

### Task 13 (wave 4): checks and final review

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -j 2 -- -D warnings`, `cargo test --workspace -j 2`.
- An opus review of the whole plan: spec coverage (§3, §4, §5 project terminal, §8), concurrency in the runtime, and the hook ack ordering.
- Stage everything. Update the memory checkpoint.
