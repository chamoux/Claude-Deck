# Claude Deck Plan 1: Spike and Core Logic — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Confirm how real Claude Code hooks behave, then build the pure, fully unit-tested logic that the helper and the window will use: hook parsing, the session state machine with its sleep rules, the `git status` parser, Claude's project-folder lookup, the saved state, and pipe framing.

**Architecture:** A Cargo workspace with two library crates and no I/O beyond files: `deck-core` (logic) and `deck-proto` (pipe framing). Nothing here starts processes or opens pipes. Plan 2 builds the helper (`deck-host`) on top. Plan 3 builds the Tauri window, the icon and the installer.

**Tech Stack:** Rust stable (MSVC toolchain), serde, serde_json, uuid, thiserror, tempfile (tests only). Spec: `docs/superpowers/specs/2026-10-19-claude-deck-design.md`.

**House rules for this repo:**
- **Never run `git commit` or `git push`.** The user commits. Every "Stage" step only runs `git add`.
- Package manager for any JS later is pnpm. Not used in this plan.
- **Shell note:** Rust is installed in Task 0 while Claude Code is already running, so its shells do not see the new PATH. Start every bash command that uses `cargo` or `rustup` with `export PATH="$HOME/.cargo/bin:$PATH" &&`.

---

## File structure

| File | Responsibility |
|---|---|
| `Cargo.toml` | Workspace: members and shared dependency versions |
| `rust-toolchain.toml` | Pins the stable MSVC toolchain with clippy and rustfmt |
| `crates/deck-core/Cargo.toml` | Core crate manifest |
| `crates/deck-core/src/lib.rs` | Module list |
| `crates/deck-core/src/hook.rs` | Parses the JSON Claude sends to a hook command into a `HookCall` |
| `crates/deck-core/src/state.rs` | `SessionState`, `SessionStatus`: transitions and the auto-sleep rule |
| `crates/deck-core/src/git.rs` | Parses `git status --porcelain=v1 -z` into `Change`s, sorts and caps them |
| `crates/deck-core/src/claude_dir.rs` | Claude's folder name for a project, finding it, listing past sessions |
| `crates/deck-core/src/store.rs` | `SavedState` (projects, sessions, settings), atomic load and save |
| `crates/deck-proto/Cargo.toml` | Proto crate manifest |
| `crates/deck-proto/src/lib.rs` | `Frame`, `encode`, `FrameDecoder` |
| `spike/log-hook.ps1`, `spike/hooks.json` | Spike only: logs every hook call from a real Claude |
| `spike/FINDINGS.md` | What the spike showed, and any change it forces on the code |

---

### Task 0: Install Rust and create the workspace

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `crates/deck-core/Cargo.toml`, `crates/deck-core/src/lib.rs`, `crates/deck-proto/Cargo.toml`, `crates/deck-proto/src/lib.rs`

- [ ] **Step 1: Install rustup**

Run: `winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements`
Expected: "Successfully installed". (MSVC Build Tools 2026 are already on this VM.)

- [ ] **Step 2: Check the toolchain**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && rustup default stable && rustc --version && cargo --version && rustup show active-toolchain`
Expected: versions print and the active toolchain ends in `x86_64-pc-windows-msvc`.

- [ ] **Step 3: Write the workspace files**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/deck-core", "crates/deck-proto"]

[workspace.package]
version = "0.1.0"
edition = "2021"
publish = false

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
thiserror = "2"
tempfile = "3"
deck-core = { path = "crates/deck-core" }
deck-proto = { path = "crates/deck-proto" }
```

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
components = ["clippy", "rustfmt"]
targets = ["x86_64-pc-windows-msvc"]
```

`crates/deck-core/Cargo.toml`:
```toml
[package]
name = "deck-core"
version.workspace = true
edition.workspace = true
publish.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
uuid.workspace = true
thiserror.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`crates/deck-core/src/lib.rs`:
```rust
//! Pure logic for Claude Deck: no processes, no pipes, no UI.
```

`crates/deck-proto/Cargo.toml`:
```toml
[package]
name = "deck-proto"
version.workspace = true
edition.workspace = true
publish.workspace = true

[dependencies]
thiserror.workspace = true
```

`crates/deck-proto/src/lib.rs`:
```rust
//! Framing for the pipe between the window, the helper and the hook command.
```

- [ ] **Step 4: Build the empty workspace**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --workspace`
Expected: both crates compile; "test result: ok. 0 passed" for each.

- [ ] **Step 5: Stage**

```bash
git add Cargo.toml rust-toolchain.toml crates/ Cargo.lock
```

---

### Task 1: Spike — how real Claude hooks behave

No production code. The result is `spike/FINDINGS.md`, which Task 2 and Task 3 depend on.

**Files:**
- Create: `spike/log-hook.ps1`, `spike/hooks.json`, `spike/FINDINGS.md`

- [ ] **Step 1: Write the hook logger**

`spike/log-hook.ps1`:
```powershell
# Spike only. Appends one line per Claude hook call:
# time, the CLAUDE_DECK_SESSION variable, and the raw JSON from standard input.
$raw = [Console]::In.ReadToEnd()
$line = '{0} deck={1} {2}' -f (Get-Date -Format o), $env:CLAUDE_DECK_SESSION, ($raw -replace "`r?`n", ' ')
Add-Content -Path (Join-Path $PSScriptRoot 'hooks.log') -Value $line -Encoding utf8
```

`spike/hooks.json` (every event runs the same logger):
```json
{
  "hooks": {
    "SessionStart":     [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File C:\\Users\\Chamoux\\Desktop\\claude-deck\\spike\\log-hook.ps1" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File C:\\Users\\Chamoux\\Desktop\\claude-deck\\spike\\log-hook.ps1" }] }],
    "PreToolUse":       [{ "matcher": "*", "hooks": [{ "type": "command", "command": "powershell -NoProfile -File C:\\Users\\Chamoux\\Desktop\\claude-deck\\spike\\log-hook.ps1" }] }],
    "PostToolUse":      [{ "matcher": "*", "hooks": [{ "type": "command", "command": "powershell -NoProfile -File C:\\Users\\Chamoux\\Desktop\\claude-deck\\spike\\log-hook.ps1" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File C:\\Users\\Chamoux\\Desktop\\claude-deck\\spike\\log-hook.ps1" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File C:\\Users\\Chamoux\\Desktop\\claude-deck\\spike\\log-hook.ps1" }] }],
    "SessionEnd":       [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -File C:\\Users\\Chamoux\\Desktop\\claude-deck\\spike\\log-hook.ps1" }] }]
  }
}
```

Add `spike/hooks.log` to `.gitignore` (append the line `spike/hooks.log`).

- [ ] **Step 2: Automatic run (print mode, a few tokens)**

```bash
mkdir -p /c/tmp/deck-spike && cd /c/tmp/deck-spike
ID=$(powershell -NoProfile -Command "[guid]::NewGuid().ToString()")
echo "$ID" > /c/tmp/deck-spike/id.txt
CLAUDE_DECK_SESSION=spike-1 claude -p "Reply with the single word: ok" --session-id "$ID" --settings 'C:\Users\Chamoux\Desktop\claude-deck\spike\hooks.json'
CLAUDE_DECK_SESSION=spike-1 claude -p "Reply with the single word: again" --resume "$ID" --settings 'C:\Users\Chamoux\Desktop\claude-deck\spike\hooks.json'
cat /c/Users/Chamoux/Desktop/claude-deck/spike/hooks.log
```
Expected: both runs print a reply. The log has `SessionStart`, `UserPromptSubmit`, `Stop` and `SessionEnd` lines with `deck=spike-1`, and every `"session_id"` equals `$ID` in both runs (resume keeps the ID).

- [ ] **Step 3: Manual run by the user (about 3 minutes)**

Ask the user to open a normal terminal and run these lines one at a time:
```powershell
cd C:\tmp\deck-spike
$env:CLAUDE_DECK_SESSION = 'spike-2'
claude --session-id ([guid]::NewGuid()) --settings C:\Users\Chamoux\Desktop\claude-deck\spike\hooks.json
```
Then, inside Claude (if it first asks whether you trust this folder, choose yes):
1. Type `create a file hello.txt containing hi`. When the permission question appears, wait 5 seconds, then approve it.
2. Type `run a bash loop that prints 1 to 30, one number per second`. After about 5 numbers, press **Esc**.
3. Do nothing for 70 seconds.
4. Press **Ctrl+C** twice and note how fast Claude exits.

- [ ] **Step 4: Write `spike/FINDINGS.md`**

Read `spike/hooks.log` and answer each question with the exact field names and values from the log:
1. Does `CLAUDE_DECK_SESSION` reach the hook? Does `session_id` equal the `--session-id` value, and stay the same after `--resume`?
2. The `Notification` payload for the permission question: its `notification_type` (if any) and `message`.
3. The `Notification` payload after 70 idle seconds: its `notification_type` (if any) and `message`.
4. After Esc: did `Stop` fire? Did any `Notification` follow?
5. On Ctrl+C twice: did `SessionEnd` fire, and how long did the exit take?
6. Any hook event or field that differs from spec §4.2, and the change it needs in Task 2 or Task 3.

- [ ] **Step 5: Apply findings**

If answer 2 or 3 differs from what `classify_notification` in Task 2 expects (`notification_type == "idle_prompt"` means waiting, anything else means needs-you, and with no type the message text decides), change Task 2's code and tests in this plan file to match the real payloads before starting Task 2. If answer 4 or 5 differs from §4.2, update the spec table.

- [ ] **Step 6: Stage**

```bash
git add spike/log-hook.ps1 spike/hooks.json spike/FINDINGS.md .gitignore docs/
```

---

### Task 2: Hook parsing (`deck-core::hook`)

**Files:**
- Create: `crates/deck-core/src/hook.rs`
- Modify: `crates/deck-core/src/lib.rs`

- [ ] **Step 1: Register the module**

`crates/deck-core/src/lib.rs`:
```rust
//! Pure logic for Claude Deck: no processes, no pipes, no UI.

pub mod hook;
```

- [ ] **Step 2: Write the failing tests**

`crates/deck-core/src/hook.rs` (tests first; the implementation follows in Step 4):
```rust
//! Parses the JSON that Claude Code sends to a hook command on standard input.

#[cfg(test)]
mod tests {
    use super::*;

    fn event(json: &str) -> HookEvent {
        parse_hook(json).expect("valid hook json").event
    }

    #[test]
    fn reads_session_id_and_prompt() {
        let call = parse_hook(
            r#"{"session_id":"abc","hook_event_name":"UserPromptSubmit","prompt":"fix the bug"}"#,
        )
        .unwrap();
        assert_eq!(call.session_id, "abc");
        assert_eq!(
            call.event,
            HookEvent::UserPromptSubmit { prompt: "fix the bug".into() }
        );
    }

    #[test]
    fn tool_hooks_mean_tool_use() {
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"PreToolUse","tool_name":"Bash"}"#),
            HookEvent::ToolUse
        );
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"PostToolUse","tool_name":"Bash"}"#),
            HookEvent::ToolUse
        );
    }

    #[test]
    fn simple_events() {
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"SessionStart","source":"startup"}"#),
            HookEvent::SessionStart
        );
        assert_eq!(event(r#"{"session_id":"a","hook_event_name":"Stop"}"#), HookEvent::Stop);
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"SessionEnd","reason":"other"}"#),
            HookEvent::SessionEnd
        );
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"PreCompact"}"#),
            HookEvent::Other("PreCompact".into())
        );
    }

    #[test]
    fn idle_prompt_notification_is_waiting_for_input() {
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"Notification","notification_type":"idle_prompt","message":"Claude is waiting for your input"}"#),
            HookEvent::WaitingForInput
        );
    }

    #[test]
    fn permission_notification_needs_you() {
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"Notification","notification_type":"permission_prompt","message":"Claude needs your permission to use Bash"}"#),
            HookEvent::NeedsYou { message: "Claude needs your permission to use Bash".into() }
        );
    }

    #[test]
    fn notification_without_type_falls_back_to_message_text() {
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"Notification","message":"Claude is waiting for your input"}"#),
            HookEvent::WaitingForInput
        );
        assert_eq!(
            event(r#"{"session_id":"a","hook_event_name":"Notification","message":"Claude needs your permission to use Write"}"#),
            HookEvent::NeedsYou { message: "Claude needs your permission to use Write".into() }
        );
    }

    #[test]
    fn unknown_notification_type_needs_you_so_it_never_sleeps() {
        assert!(matches!(
            event(r#"{"session_id":"a","hook_event_name":"Notification","notification_type":"elicitation_dialog","message":"Pick one"}"#),
            HookEvent::NeedsYou { .. }
        ));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(parse_hook("not json"), Err(HookParseError::NotJson(_))));
        assert_eq!(
            parse_hook(r#"{"hook_event_name":"Stop"}"#),
            Err(HookParseError::MissingField("session_id"))
        );
        assert_eq!(
            parse_hook(r#"{"session_id":"a"}"#),
            Err(HookParseError::MissingField("hook_event_name"))
        );
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core hook`
Expected: compile errors: `parse_hook`, `HookEvent`, `HookParseError` not found.

- [ ] **Step 4: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/deck-core/src/hook.rs`:
```rust
use serde_json::Value;

/// What one hook call tells us about a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit { prompt: String },
    /// PreToolUse or PostToolUse: Claude is running a tool.
    ToolUse,
    /// A Notification asking for permission or asking the user a question.
    NeedsYou { message: String },
    /// A Notification saying Claude has been waiting for the next prompt.
    WaitingForInput,
    Stop,
    SessionEnd,
    /// Any other hook event. The state machine ignores it.
    Other(String),
}

/// One parsed hook call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookCall {
    /// Claude's session id from the payload.
    pub session_id: String,
    pub event: HookEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HookParseError {
    #[error("hook input is not JSON: {0}")]
    NotJson(String),
    #[error("hook input has no {0} field")]
    MissingField(&'static str),
}

/// Parses the JSON Claude writes to a hook command's standard input.
pub fn parse_hook(input: &str) -> Result<HookCall, HookParseError> {
    let v: Value =
        serde_json::from_str(input).map_err(|e| HookParseError::NotJson(e.to_string()))?;
    let name = str_field(&v, "hook_event_name")
        .ok_or(HookParseError::MissingField("hook_event_name"))?;
    let session_id = str_field(&v, "session_id")
        .ok_or(HookParseError::MissingField("session_id"))?
        .to_string();
    let event = match name {
        "SessionStart" => HookEvent::SessionStart,
        "UserPromptSubmit" => HookEvent::UserPromptSubmit {
            prompt: str_field(&v, "prompt").unwrap_or("").to_string(),
        },
        "PreToolUse" | "PostToolUse" => HookEvent::ToolUse,
        "Notification" => classify_notification(&v),
        "Stop" => HookEvent::Stop,
        "SessionEnd" => HookEvent::SessionEnd,
        other => HookEvent::Other(other.to_string()),
    };
    Ok(HookCall { session_id, event })
}

/// "Waiting for input" only when Claude says so. Anything else counts as
/// "needs you", which never auto-sleeps: the safe side (spec §4.2).
fn classify_notification(v: &Value) -> HookEvent {
    let message = str_field(v, "message").unwrap_or("").to_string();
    let waiting = match str_field(v, "notification_type") {
        Some(kind) => kind == "idle_prompt",
        None => message.to_lowercase().contains("waiting for your input"),
    };
    if waiting {
        HookEvent::WaitingForInput
    } else {
        HookEvent::NeedsYou { message }
    }
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core hook`
Expected: `test result: ok. 8 passed`.

- [ ] **Step 6: Stage**

```bash
git add crates/deck-core/src/lib.rs crates/deck-core/src/hook.rs
```

---

### Task 3: Session state machine (`deck-core::state`)

**Files:**
- Create: `crates/deck-core/src/state.rs`
- Modify: `crates/deck-core/src/lib.rs`

- [ ] **Step 1: Register the module**

`crates/deck-core/src/lib.rs`:
```rust
//! Pure logic for Claude Deck: no processes, no pipes, no UI.

pub mod hook;
pub mod state;
```

- [ ] **Step 2: Write the failing tests**

`crates/deck-core/src/state.rs`:
```rust
//! Session states and the auto-sleep rule (spec §4.2).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook::HookEvent;

    const MIN: i64 = 60_000;
    const T0: i64 = 1_000_000;

    fn idle_at(t: i64) -> SessionStatus {
        let mut s = SessionStatus::started();
        s.apply(&HookEvent::Stop, t);
        s
    }

    #[test]
    fn a_new_process_counts_as_working_and_never_auto_sleeps() {
        let s = SessionStatus::started();
        assert_eq!(s.state, SessionState::Working);
        assert!(!s.should_auto_sleep(T0 + 600 * MIN, Some(30)));
    }

    #[test]
    fn session_start_prompt_and_stop() {
        let mut s = SessionStatus::started();
        s.apply(&HookEvent::SessionStart, T0);
        assert_eq!(s, SessionStatus { state: SessionState::Idle, idle_since_ms: Some(T0) });
        s.apply(&HookEvent::UserPromptSubmit { prompt: "go".into() }, T0 + 1);
        assert_eq!(s, SessionStatus { state: SessionState::Working, idle_since_ms: None });
        s.apply(&HookEvent::Stop, T0 + 2);
        assert_eq!(s, SessionStatus { state: SessionState::Idle, idle_since_ms: Some(T0 + 2) });
    }

    #[test]
    fn idle_sleeps_after_the_set_minutes() {
        let s = idle_at(T0);
        assert!(!s.should_auto_sleep(T0 + 30 * MIN - 1, Some(30)));
        assert!(s.should_auto_sleep(T0 + 30 * MIN, Some(30)));
    }

    #[test]
    fn auto_sleep_off_never_sleeps() {
        assert!(!idle_at(T0).should_auto_sleep(T0 + 10_000 * MIN, None));
    }

    #[test]
    fn needs_you_never_sleeps_and_tool_use_clears_it() {
        let mut s = SessionStatus::started();
        s.apply(&HookEvent::NeedsYou { message: "permission".into() }, T0);
        assert_eq!(s.state, SessionState::NeedsYou);
        assert!(!s.should_auto_sleep(T0 + 10_000 * MIN, Some(30)));
        s.apply(&HookEvent::ToolUse, T0 + 1);
        assert_eq!(s.state, SessionState::Working);
    }

    #[test]
    fn waiting_for_input_recovers_a_session_interrupted_with_esc() {
        let mut s = SessionStatus::started();
        s.apply(&HookEvent::UserPromptSubmit { prompt: "count".into() }, T0);
        // Esc: no Stop arrives. A minute later Claude says it is waiting.
        s.apply(&HookEvent::WaitingForInput, T0 + MIN);
        assert_eq!(s, SessionStatus { state: SessionState::Idle, idle_since_ms: Some(T0 + MIN) });
    }

    #[test]
    fn waiting_for_input_keeps_an_existing_idle_clock() {
        let mut s = idle_at(T0);
        s.apply(&HookEvent::WaitingForInput, T0 + MIN);
        assert_eq!(s.idle_since_ms, Some(T0));
    }

    #[test]
    fn waiting_for_input_does_not_clear_needs_you() {
        let mut s = SessionStatus::started();
        s.apply(&HookEvent::NeedsYou { message: "permission".into() }, T0);
        s.apply(&HookEvent::WaitingForInput, T0 + MIN);
        assert_eq!(s.state, SessionState::NeedsYou);
    }

    #[test]
    fn typing_restarts_the_idle_clock_only_when_idle() {
        let mut s = idle_at(T0);
        s.on_input(T0 + 20 * MIN);
        assert_eq!(s.idle_since_ms, Some(T0 + 20 * MIN));
        assert!(!s.should_auto_sleep(T0 + 30 * MIN, Some(30)));

        let mut w = SessionStatus::started();
        w.on_input(T0);
        assert_eq!(w, SessionStatus::started());
    }

    #[test]
    fn session_end_or_exit_means_asleep_and_later_hooks_are_ignored() {
        let mut s = idle_at(T0);
        s.apply(&HookEvent::SessionEnd, T0 + 1);
        assert_eq!(s, SessionStatus::asleep());
        s.apply(&HookEvent::UserPromptSubmit { prompt: "late".into() }, T0 + 2);
        assert_eq!(s, SessionStatus::asleep());

        let mut e = SessionStatus::started();
        e.on_exit();
        assert_eq!(e, SessionStatus::asleep());
    }

    #[test]
    fn other_events_change_nothing() {
        let mut s = idle_at(T0);
        s.apply(&HookEvent::Other("PreCompact".into()), T0 + 5);
        assert_eq!(s, idle_at(T0));
    }

    #[test]
    fn busy_and_sleep_without_asking() {
        assert!(SessionStatus::started().is_busy());
        assert!(!SessionStatus::started().can_sleep_without_asking());
        assert!(!idle_at(T0).is_busy());
        assert!(idle_at(T0).can_sleep_without_asking());
        assert!(!SessionStatus::asleep().is_busy());
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core state`
Expected: compile errors: `SessionStatus`, `SessionState` not found.

- [ ] **Step 4: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/deck-core/src/state.rs`:
```rust
use crate::hook::HookEvent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Working,
    NeedsYou,
    Idle,
    Asleep,
}

/// Live status of one session. Times are Unix milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStatus {
    pub state: SessionState,
    /// While Idle: when the idle clock started.
    pub idle_since_ms: Option<i64>,
}

impl SessionStatus {
    /// A Claude process that was just started. It counts as Working until a
    /// hook says otherwise, so it can never be auto-slept by mistake.
    pub fn started() -> Self {
        Self { state: SessionState::Working, idle_since_ms: None }
    }

    pub fn asleep() -> Self {
        Self { state: SessionState::Asleep, idle_since_ms: None }
    }

    /// Applies one hook event. Hooks that arrive after the session ended are ignored.
    pub fn apply(&mut self, event: &HookEvent, now_ms: i64) {
        if self.state == SessionState::Asleep {
            return;
        }
        match event {
            HookEvent::SessionStart | HookEvent::Stop => self.set_idle(now_ms),
            HookEvent::WaitingForInput => {
                // Never overrides a pending question, and keeps a running idle clock.
                if self.state == SessionState::Working {
                    self.set_idle(now_ms);
                }
            }
            HookEvent::UserPromptSubmit { .. } | HookEvent::ToolUse => {
                self.set(SessionState::Working)
            }
            HookEvent::NeedsYou { .. } => self.set(SessionState::NeedsYou),
            HookEvent::SessionEnd => *self = Self::asleep(),
            HookEvent::Other(_) => {}
        }
    }

    /// Keyboard input to an Idle session restarts its idle clock.
    pub fn on_input(&mut self, now_ms: i64) {
        if self.state == SessionState::Idle {
            self.idle_since_ms = Some(now_ms);
        }
    }

    /// The Claude process ended: by itself, by sleeping, or by a crash.
    pub fn on_exit(&mut self) {
        *self = Self::asleep();
    }

    /// True when an Idle session has been idle for `auto_sleep_minutes`.
    /// `None` means auto-sleep is off.
    pub fn should_auto_sleep(&self, now_ms: i64, auto_sleep_minutes: Option<u32>) -> bool {
        match (self.state, self.idle_since_ms, auto_sleep_minutes) {
            (SessionState::Idle, Some(since), Some(minutes)) => {
                now_ms - since >= i64::from(minutes) * 60_000
            }
            _ => false,
        }
    }

    /// "Sleep now" may skip the confirmation only for Idle sessions.
    pub fn can_sleep_without_asking(&self) -> bool {
        self.state == SessionState::Idle
    }

    /// Working or Needs you: closing the window shows the dialog (spec §4.4).
    pub fn is_busy(&self) -> bool {
        matches!(self.state, SessionState::Working | SessionState::NeedsYou)
    }

    fn set_idle(&mut self, now_ms: i64) {
        self.state = SessionState::Idle;
        self.idle_since_ms = Some(now_ms);
    }

    fn set(&mut self, state: SessionState) {
        self.state = state;
        self.idle_since_ms = None;
    }
}
```

Note: `WaitingForInput` only changes Working to Idle. From Idle it keeps the clock, and from Needs you it changes nothing. That is what the three `waiting_for_input_*` tests check.

- [ ] **Step 5: Run the tests to see them pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core state`
Expected: `test result: ok. 12 passed`.

- [ ] **Step 6: Stage**

```bash
git add crates/deck-core/src/lib.rs crates/deck-core/src/state.rs
```

---

### Task 4: `git status` parser (`deck-core::git`)

**Files:**
- Create: `crates/deck-core/src/git.rs`
- Modify: `crates/deck-core/src/lib.rs`

- [ ] **Step 1: Register the module**

`crates/deck-core/src/lib.rs`:
```rust
//! Pure logic for Claude Deck: no processes, no pipes, no UI.

pub mod git;
pub mod hook;
pub mod state;
```

- [ ] **Step 2: Write the failing tests**

`crates/deck-core/src/git.rs`:
```rust
//! Parses `git status --porcelain=v1 -z --untracked-files=all` for the Changes tab (spec §6).

#[cfg(test)]
mod tests {
    use super::*;

    fn change(kind: ChangeKind, path: &str) -> Change {
        Change { kind, path: path.into(), old_path: None, conflict: false }
    }

    #[test]
    fn maps_every_status_code() {
        let out = b"?? new.txt\0 M app/a.php\0M  app/b.php\0D  old.php\0 D gone.php\0R  new/name.php\0old/name.php\0UU both.php\0A  added.php\0AM added2.php\0AD ghost.php\0 T link\0C  copy.php\0orig.php\0";
        assert_eq!(
            parse_porcelain_z(out),
            vec![
                change(ChangeKind::Created, "new.txt"),
                change(ChangeKind::Modified, "app/a.php"),
                change(ChangeKind::Modified, "app/b.php"),
                change(ChangeKind::Deleted, "old.php"),
                change(ChangeKind::Deleted, "gone.php"),
                Change {
                    kind: ChangeKind::Renamed,
                    path: "new/name.php".into(),
                    old_path: Some("old/name.php".into()),
                    conflict: false,
                },
                Change { kind: ChangeKind::Modified, path: "both.php".into(), old_path: None, conflict: true },
                change(ChangeKind::Created, "added.php"),
                change(ChangeKind::Created, "added2.php"),
                change(ChangeKind::Modified, "link"),
                change(ChangeKind::Created, "copy.php"),
            ]
        );
    }

    #[test]
    fn paths_with_spaces_and_accents_survive() {
        let out = "?? docs/my notes é.md\0".as_bytes();
        assert_eq!(parse_porcelain_z(out), vec![change(ChangeKind::Created, "docs/my notes é.md")]);
    }

    #[test]
    fn empty_and_short_entries_are_skipped() {
        assert!(parse_porcelain_z(b"").is_empty());
        assert!(parse_porcelain_z(b"?? \0\0").is_empty());
    }

    #[test]
    fn summarize_sorts_by_kind_then_path_and_caps() {
        let list = summarize(
            vec![
                change(ChangeKind::Deleted, "z.txt"),
                change(ChangeKind::Created, "b.txt"),
                change(ChangeKind::Modified, "m.txt"),
                change(ChangeKind::Created, "a.txt"),
            ],
            3,
        );
        assert_eq!(list.total, 4);
        assert_eq!(
            list.items,
            vec![
                change(ChangeKind::Created, "a.txt"),
                change(ChangeKind::Created, "b.txt"),
                change(ChangeKind::Modified, "m.txt"),
            ]
        );
    }

    #[test]
    fn split_path_gives_name_and_folder() {
        assert_eq!(split_path("app/Services/InvoiceService.php"), ("InvoiceService.php", "app/Services"));
        assert_eq!(split_path("README.md"), ("README.md", ""));
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core git`
Expected: compile errors: `parse_porcelain_z`, `Change`, `ChangeKind`, `summarize`, `split_path` not found.

- [ ] **Step 4: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/deck-core/src/git.rs`:
```rust
use serde::Serialize;

/// Lines shown before "and N more" (spec §6).
pub const DEFAULT_CAP: usize = 500;

/// Order here is the order of the groups in the Changes tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub kind: ChangeKind,
    /// Relative to the repo root, with forward slashes.
    pub path: String,
    /// The previous path, for renames only.
    pub old_path: Option<String>,
    /// A merge conflict, shown as Modified with the word "conflict".
    pub conflict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeList {
    /// Sorted by kind, then path. At most the cap.
    pub items: Vec<Change>,
    /// How many changes there were before the cap.
    pub total: usize,
}

/// Parses the output of `git status --porcelain=v1 -z --untracked-files=all`.
/// Each entry is "XY path\0". Renames and copies are followed by "old path\0".
pub fn parse_porcelain_z(out: &[u8]) -> Vec<Change> {
    let text = String::from_utf8_lossy(out);
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut changes = Vec::new();
    while let Some(entry) = fields.next() {
        if entry.len() < 4 || !entry.is_char_boundary(3) {
            continue;
        }
        let (code, path) = entry.split_at(3);
        let mut chars = code.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');
        let old_path = if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            fields.next().map(str::to_string)
        } else {
            None
        };
        if let Some(change) = classify(x, y, path.to_string(), old_path) {
            changes.push(change);
        }
    }
    changes
}

fn classify(x: char, y: char, path: String, old_path: Option<String>) -> Option<Change> {
    let conflict = matches!(
        (x, y),
        ('D', 'D') | ('A', 'U') | ('U', 'D') | ('U', 'A') | ('D', 'U') | ('A', 'A') | ('U', 'U')
    );
    let kind = if conflict {
        ChangeKind::Modified
    } else if x == '?' && y == '?' {
        ChangeKind::Created
    } else if x == '!' {
        return None;
    } else if x == 'R' || y == 'R' {
        ChangeKind::Renamed
    } else if x == 'C' || y == 'C' {
        ChangeKind::Created
    } else if x == 'A' && y == 'D' {
        // Added, then deleted again before any commit: nothing to show.
        return None;
    } else if x == 'D' || y == 'D' {
        ChangeKind::Deleted
    } else if x == 'A' || y == 'A' {
        // Y='A' is intent-to-add (`git add -N`).
        ChangeKind::Created
    } else if matches!(x, 'M' | 'T') || matches!(y, 'M' | 'T') {
        ChangeKind::Modified
    } else {
        return None;
    };
    let old_path = if kind == ChangeKind::Renamed { old_path } else { None };
    Some(Change { kind, path, old_path, conflict })
}

/// Sorts by kind, then path, and keeps at most `cap` items.
pub fn summarize(mut changes: Vec<Change>, cap: usize) -> ChangeList {
    changes.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.path.cmp(&b.path)));
    let total = changes.len();
    changes.truncate(cap);
    ChangeList { items: changes, total }
}

/// "app/Services/InvoiceService.php" → ("InvoiceService.php", "app/Services").
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((folder, name)) => (name, folder),
        None => (path, ""),
    }
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core git`
Expected: `test result: ok. 6 passed` (5 tests above plus `intent_to_add_and_mixed_codes`, added after review: ` A`, `MD`, `RM`, `!!`).

- [ ] **Step 6: Stage**

```bash
git add crates/deck-core/src/lib.rs crates/deck-core/src/git.rs
```

---

### Task 5: Claude's project folders and past sessions (`deck-core::claude_dir`)

Facts from this machine (2026-10-19): conversations are `~/.claude/projects/<folder>/<session id>.jsonl`. `<folder>` is the project path with every non-alphanumeric character replaced by `-`, and the drive letter may be `C` or `c`. Windows folders are case-insensitive, so at most one folder matches. Lines with `"type":"last-prompt"` carry the prompt in `"lastPrompt"`, but some of those lines omit it. Message lines carry `"cwd"`. Transcripts can be several MB, so only the first 32 KB and the last 64 KB are read.

**Files:**
- Create: `crates/deck-core/src/claude_dir.rs`
- Modify: `crates/deck-core/src/lib.rs`

- [ ] **Step 1: Register the module**

`crates/deck-core/src/lib.rs`:
```rust
//! Pure logic for Claude Deck: no processes, no pipes, no UI.

pub mod claude_dir;
pub mod git;
pub mod hook;
pub mod state;
```

- [ ] **Step 2: Write the failing tests**

`crates/deck-core/src/claude_dir.rs`:
```rust
//! Finds a project's Claude conversations under ~/.claude/projects (spec §4.7).

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::time::{Duration, SystemTime};

    #[test]
    fn encodes_like_claude_does() {
        assert_eq!(encode_project_dir(r"C:\Users\Chamoux\Desktop\CodeHive"), "C--Users-Chamoux-Desktop-CodeHive");
        assert_eq!(
            encode_project_dir(r"c:\Users\Chamoux\Desktop\New folder (2)"),
            "c--Users-Chamoux-Desktop-New-folder--2-"
        );
        assert_eq!(encode_project_dir(r"C:\Users\Chamoux\.claude\skills"), "C--Users-Chamoux--claude-skills");
    }

    #[test]
    fn same_path_ignores_case_slashes_and_trailing_slash() {
        assert!(same_path(r"C:\Users\Me\App", "c:/users/me/app/"));
        assert!(!same_path(r"C:\Users\Me\App", r"C:\Users\Me\App2"));
    }

    #[test]
    fn finds_the_folder_whatever_the_drive_letter_case() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("c--Users-Me-Desktop-App")).unwrap();
        let found = find_project_dir(root.path(), r"C:\Users\Me\Desktop\App").unwrap();
        assert_eq!(found.file_name().unwrap(), "c--Users-Me-Desktop-App");
        assert!(find_project_dir(root.path(), r"C:\Users\Me\Desktop\Nope").is_none());
    }

    fn write_transcript(dir: &Path, id: &str, cwd: &str, prompt: &str, age_secs: u64) {
        let path = dir.join(format!("{id}.jsonl"));
        let cwd = serde_json::to_string(cwd).unwrap();
        let prompt = serde_json::to_string(prompt).unwrap();
        fs::write(
            &path,
            format!(
                "{{\"type\":\"last-prompt\",\"sessionId\":\"{id}\"}}\n{{\"type\":\"user\",\"cwd\":{cwd}}}\n{{\"type\":\"last-prompt\",\"lastPrompt\":{prompt}}}\n"
            ),
        )
        .unwrap();
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
    }

    #[test]
    fn lists_past_sessions_newest_first_skipping_open_and_foreign_ones() {
        let root = tempfile::tempdir().unwrap();
        let project = r"C:\Users\Me\Desktop\My App";
        let dir = root.path().join("c--Users-Me-Desktop-My-App");
        fs::create_dir(&dir).unwrap();
        write_transcript(&dir, "old", project, "first idea", 300);
        write_transcript(&dir, "new", project, "second idea", 10);
        write_transcript(&dir, "open-already", project, "skip me", 5);
        // "My-App" encodes to the same folder name as "My App"; its cwd tells them apart.
        write_transcript(&dir, "other-folder", r"C:\Users\Me\Desktop\My-App", "not this project", 1);
        fs::write(dir.join("notes.txt"), "ignored").unwrap();

        let exclude = HashSet::from(["open-already".to_string()]);
        let got = list_past_sessions(root.path(), project, &exclude);
        let ids: Vec<&str> = got.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["new", "old"]);
        assert_eq!(got[0].last_prompt.as_deref(), Some("second idea"));
        assert!(got[0].modified_ms > got[1].modified_ms);
    }

    #[test]
    fn reads_cwd_and_latest_prompt_from_head_and_tail_of_a_big_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.jsonl");
        let mut text = String::from(
            "{\"type\":\"user\",\"cwd\":\"C:\\\\Work\\\\App\"}\n{\"type\":\"last-prompt\",\"lastPrompt\":\"early\"}\n",
        );
        let filler = format!("{{\"type\":\"assistant\",\"text\":\"{}\"}}\n", "x".repeat(1000));
        for _ in 0..500 {
            text.push_str(&filler); // about 500 KB: bigger than head + tail
        }
        text.push_str("{\"type\":\"last-prompt\",\"lastPrompt\":\"latest\"}\n{\"type\":\"last-prompt\",\"leafUuid\":\"x\"}\n");
        fs::write(&path, text).unwrap();

        let s = summarize_file(&path).unwrap();
        assert_eq!(s.cwd.as_deref(), Some(r"C:\Work\App"));
        assert_eq!(s.last_prompt.as_deref(), Some("latest"));
    }

    #[test]
    fn missing_root_gives_nothing() {
        let got = list_past_sessions(Path::new(r"C:\does\not\exist"), r"C:\Work\App", &HashSet::new());
        assert!(got.is_empty());
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core claude_dir`
Expected: compile errors: `encode_project_dir`, `same_path`, `find_project_dir`, `list_past_sessions`, `summarize_file` not found.

- [ ] **Step 4: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/deck-core/src/claude_dir.rs`:
```rust
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const HEAD_BYTES: u64 = 32 * 1024;
const TAIL_BYTES: u64 = 64 * 1024;

/// A conversation that can be reopened from "Resume past session…".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PastSession {
    pub id: String,
    pub last_prompt: Option<String>,
    pub modified_ms: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TranscriptSummary {
    pub cwd: Option<String>,
    pub last_prompt: Option<String>,
}

/// Claude's folder name for a project: every character that is not an ASCII
/// letter or digit becomes '-'.
pub fn encode_project_dir(project_path: &str) -> String {
    project_path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Windows paths: compare without case, with either slash, without a trailing slash.
pub fn same_path(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

/// The folder under `projects_root` for this project, ignoring case.
pub fn find_project_dir(projects_root: &Path, project_path: &str) -> Option<PathBuf> {
    let wanted = encode_project_dir(project_path).to_ascii_lowercase();
    fs::read_dir(projects_root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .find(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == wanted)
        .map(|e| e.path())
}

/// Conversations for `project_path`, newest first. Skips ids in `exclude`
/// (already open in the app) and files whose `cwd` is a different folder.
pub fn list_past_sessions(
    projects_root: &Path,
    project_path: &str,
    exclude: &HashSet<String>,
) -> Vec<PastSession> {
    let Some(dir) = find_project_dir(projects_root, project_path) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        if exclude.contains(&id) {
            continue;
        }
        let Ok(summary) = summarize_file(&path) else {
            continue;
        };
        if let Some(cwd) = &summary.cwd {
            if !same_path(cwd, project_path) {
                continue;
            }
        }
        let modified_ms = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        out.push(PastSession { id, last_prompt: summary.last_prompt, modified_ms });
    }
    out.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    out
}

/// Reads only the head and tail of a transcript, then summarizes them.
pub fn summarize_file(path: &Path) -> io::Result<TranscriptSummary> {
    Ok(summarize_transcript(&read_head_and_tail(path)?))
}

/// The first `cwd` and the last non-empty `lastPrompt`. Lines that are not
/// valid JSON (for example, cut at the head/tail boundary) are skipped.
pub fn summarize_transcript(text: &str) -> TranscriptSummary {
    let mut s = TranscriptSummary::default();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if s.cwd.is_none() {
            if let Some(cwd) = v.get("cwd").and_then(Value::as_str) {
                s.cwd = Some(cwd.to_string());
            }
        }
        if v.get("type").and_then(Value::as_str) == Some("last-prompt") {
            if let Some(p) = v.get("lastPrompt").and_then(Value::as_str) {
                if !p.is_empty() {
                    s.last_prompt = Some(p.to_string());
                }
            }
        }
    }
    s
}

fn read_head_and_tail(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut bytes = Vec::new();
    if len <= HEAD_BYTES + TAIL_BYTES {
        file.read_to_end(&mut bytes)?;
    } else {
        bytes.resize(HEAD_BYTES as usize, 0);
        file.read_exact(&mut bytes)?;
        bytes.push(b'\n');
        file.seek(SeekFrom::End(-(TAIL_BYTES as i64)))?;
        file.read_to_end(&mut bytes)?;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core claude_dir`
Expected: `test result: ok. 6 passed`.

- [ ] **Step 6: Check against the real folders (read-only)**

Run: `ls /c/Users/Chamoux/.claude/projects | grep -i -- "-Desktop-CodeHive$"`
Expected: one line, `c--Users-Chamoux-Desktop-CodeHive`. That matches `encode_project_dir(r"C:\Users\Chamoux\Desktop\CodeHive")` when case is ignored.

- [ ] **Step 7: Stage**

```bash
git add crates/deck-core/src/lib.rs crates/deck-core/src/claude_dir.rs
```

---

### Task 6: Saved state (`deck-core::store`)

**Files:**
- Create: `crates/deck-core/src/store.rs`
- Modify: `crates/deck-core/src/lib.rs`

- [ ] **Step 1: Register the module**

`crates/deck-core/src/lib.rs`:
```rust
//! Pure logic for Claude Deck: no processes, no pipes, no UI.

pub mod claude_dir;
pub mod git;
pub mod hook;
pub mod state;
pub mod store;
```

- [ ] **Step 2: Write the failing tests**

`crates/deck-core/src/store.rs`:
```rust
//! The app's saved state, %LOCALAPPDATA%\ClaudeDeck\state.json (spec §7).

#[cfg(test)]
mod tests {
    use super::*;

    const ID1: &str = "11111111-1111-4111-8111-111111111111";

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let out = load(&dir.path().join("state.json"), 1).unwrap();
        assert_eq!(out.state, SavedState::default());
        assert_eq!(out.state.settings.auto_sleep_minutes, Some(30));
        assert!(out.corrupt_backup.is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("state.json");
        let mut state = SavedState::default();
        let pid = state.add_project(r"C:\Work\App").id.clone();
        state.add_session(&pid, ID1.into(), 5);
        state.settings.auto_sleep_minutes = None;
        save(&path, &state).unwrap();
        assert_eq!(load(&path, 6).unwrap().state, state);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, "{ not json").unwrap();
        let out = load(&path, 1234).unwrap();
        assert_eq!(out.state, SavedState::default());
        let backup = out.corrupt_backup.unwrap();
        assert_eq!(backup, dir.path().join("state.corrupt-1234.json"));
        assert_eq!(fs::read_to_string(backup).unwrap(), "{ not json");
        assert!(!path.exists());
    }

    #[test]
    fn auto_sleep_stays_between_5_and_240_minutes_and_null_means_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"settings":{"auto_sleep_minutes":1}}"#).unwrap();
        assert_eq!(load(&path, 1).unwrap().state.settings.auto_sleep_minutes, Some(5));
        fs::write(&path, r#"{"settings":{"auto_sleep_minutes":999}}"#).unwrap();
        assert_eq!(load(&path, 1).unwrap().state.settings.auto_sleep_minutes, Some(240));
        fs::write(&path, r#"{"settings":{"auto_sleep_minutes":null}}"#).unwrap();
        assert_eq!(load(&path, 1).unwrap().state.settings.auto_sleep_minutes, None);
    }

    #[test]
    fn adding_the_same_folder_twice_keeps_one_project() {
        let mut s = SavedState::default();
        let a = s.add_project(r"C:\Work\App").id.clone();
        let b = s.add_project("c:/work/app/").id.clone();
        assert_eq!(a, b);
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.projects[0].name, "App");
    }

    #[test]
    fn removing_a_project_removes_its_sessions() {
        let mut s = SavedState::default();
        let a = s.add_project(r"C:\Work\A").id.clone();
        let b = s.add_project(r"C:\Work\B").id.clone();
        s.add_session(&a, "s1".into(), 1);
        s.add_session(&b, "s2".into(), 1);
        s.remove_project(&a);
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.sessions.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["s2"]);
    }

    #[test]
    fn adding_an_existing_session_twice_keeps_one() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.add_session(&p, "s1".into(), 2);
        assert_eq!(s.sessions.len(), 1);
        s.remove_session("s1");
        assert!(s.sessions.is_empty());
    }

    #[test]
    fn first_prompt_names_a_session_once_and_rename_wins() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.name_from_prompt("s1", "  fix the\ninvoice   total when a discount is applied twice  ");
        assert_eq!(
            s.session("s1").unwrap().name.as_deref(),
            Some("fix the invoice total when a discount is…")
        );
        s.name_from_prompt("s1", "a later prompt");
        assert_eq!(
            s.session("s1").unwrap().name.as_deref(),
            Some("fix the invoice total when a discount is…")
        );
        s.rename("s1", "Invoice bug");
        let sess = s.session("s1").unwrap();
        assert_eq!(sess.name.as_deref(), Some("Invoice bug"));
        assert!(sess.renamed);
    }

    #[test]
    fn short_prompts_are_kept_whole_and_blank_ones_ignored() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.name_from_prompt("s1", "   ");
        assert_eq!(s.session("s1").unwrap().name, None);
        s.name_from_prompt("s1", "hello");
        assert_eq!(s.session("s1").unwrap().name.as_deref(), Some("hello"));
    }

    #[test]
    fn touch_updates_last_active() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.touch("s1", 99);
        assert_eq!(s.session("s1").unwrap().last_active_ms, 99);
    }

    #[test]
    fn new_ids_are_uuids() {
        assert!(uuid::Uuid::parse_str(&new_id()).is_ok());
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core store`
Expected: compile errors: `SavedState`, `load`, `save`, `new_id` not found.

- [ ] **Step 4: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/deck-core/src/store.rs`:
```rust
use crate::claude_dir::same_path;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const STATE_VERSION: u32 = 1;
pub const MIN_AUTO_SLEEP_MINUTES: u32 = 5;
pub const MAX_AUTO_SLEEP_MINUTES: u32 = 240;
const NAME_CHARS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedState {
    pub version: u32,
    /// In sidebar order.
    pub projects: Vec<Project>,
    /// Only sessions shown in the sidebar. Closing one removes it.
    pub sessions: Vec<SavedSession>,
    pub settings: Settings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSession {
    /// Claude's session id (a UUID), used with --session-id and --resume.
    pub id: String,
    pub project_id: String,
    pub name: Option<String>,
    /// Set once the user renames it. The first prompt then never overwrites the name.
    pub renamed: bool,
    pub created_ms: i64,
    pub last_active_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Minutes before an Idle session sleeps. `None` (JSON null) = off.
    pub auto_sleep_minutes: Option<u32>,
    /// Full path to claude.exe. `None` = find it automatically.
    pub claude_path: Option<String>,
    pub window: Option<WindowPlacement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub struct LoadOutcome {
    pub state: SavedState,
    /// Set when the file could not be read as JSON and was moved here.
    pub corrupt_backup: Option<PathBuf>,
}

impl Default for SavedState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            projects: Vec::new(),
            sessions: Vec::new(),
            settings: Settings::default(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self { auto_sleep_minutes: Some(30), claude_path: None, window: None }
    }
}

impl Settings {
    /// Keeps auto-sleep within 5–240 minutes (spec §4.2).
    pub fn normalize(&mut self) {
        self.auto_sleep_minutes = self
            .auto_sleep_minutes
            .map(|m| m.clamp(MIN_AUTO_SLEEP_MINUTES, MAX_AUTO_SLEEP_MINUTES));
    }
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl SavedState {
    /// Adds a project folder, or returns the existing one for the same folder.
    pub fn add_project(&mut self, path: &str) -> &Project {
        if let Some(i) = self.projects.iter().position(|p| same_path(&p.path, path)) {
            return &self.projects[i];
        }
        let clean = path.trim_end_matches(['\\', '/']);
        let name = Path::new(clean)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| clean.to_string());
        self.projects.push(Project { id: new_id(), path: clean.to_string(), name });
        self.projects.last().expect("just pushed")
    }

    /// Removes the project and its sessions from the app. Files are not touched.
    pub fn remove_project(&mut self, project_id: &str) {
        self.projects.retain(|p| p.id != project_id);
        self.sessions.retain(|s| s.project_id != project_id);
    }

    /// Adds a session with Claude's id. Adding the same id again keeps the first.
    pub fn add_session(&mut self, project_id: &str, id: String, now_ms: i64) -> &SavedSession {
        if let Some(i) = self.sessions.iter().position(|s| s.id == id) {
            return &self.sessions[i];
        }
        self.sessions.push(SavedSession {
            id,
            project_id: project_id.to_string(),
            name: None,
            renamed: false,
            created_ms: now_ms,
            last_active_ms: now_ms,
        });
        self.sessions.last().expect("just pushed")
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.sessions.retain(|s| s.id != session_id);
    }

    pub fn session(&self, session_id: &str) -> Option<&SavedSession> {
        self.sessions.iter().find(|s| s.id == session_id)
    }

    /// Names an unnamed session after its first prompt, cut to 40 characters.
    pub fn name_from_prompt(&mut self, session_id: &str, prompt: &str) {
        let Some(s) = self.session_mut(session_id) else { return };
        if s.renamed || s.name.is_some() {
            return;
        }
        let text = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return;
        }
        s.name = Some(if text.chars().count() > NAME_CHARS {
            let cut: String = text.chars().take(NAME_CHARS).collect();
            format!("{}…", cut.trim_end())
        } else {
            text
        });
    }

    pub fn rename(&mut self, session_id: &str, name: &str) {
        if let Some(s) = self.session_mut(session_id) {
            s.name = Some(name.trim().to_string());
            s.renamed = true;
        }
    }

    pub fn touch(&mut self, session_id: &str, now_ms: i64) {
        if let Some(s) = self.session_mut(session_id) {
            s.last_active_ms = now_ms;
        }
    }

    fn session_mut(&mut self, session_id: &str) -> Option<&mut SavedSession> {
        self.sessions.iter_mut().find(|s| s.id == session_id)
    }
}

/// Loads the state. A missing file gives defaults. A file that is not valid
/// JSON is moved to "<name>.corrupt-<now_ms>.json" and defaults are returned.
pub fn load(path: &Path, now_ms: i64) -> io::Result<LoadOutcome> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(LoadOutcome { state: SavedState::default(), corrupt_backup: None })
        }
        Err(e) => return Err(e),
    };
    match serde_json::from_slice::<SavedState>(&bytes) {
        Ok(mut state) => {
            state.settings.normalize();
            Ok(LoadOutcome { state, corrupt_backup: None })
        }
        Err(_) => {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("state");
            let backup = path.with_file_name(format!("{stem}.corrupt-{now_ms}.json"));
            fs::rename(path, &backup)?;
            Ok(LoadOutcome { state: SavedState::default(), corrupt_backup: Some(backup) })
        }
    }
}

/// Writes to "<file>.tmp", flushes it to disk, then renames it over the old
/// file, so a crash never leaves a half-written state.
pub fn save(path: &Path, state: &SavedState) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let mut file = fs::File::create(&tmp)?;
    file.write_all(&json)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-core store`
Expected: `test result: ok. 11 passed`.

**Changes made after review (in the code, 16 tests):**
- `#[serde(default)]` on `Project.name` and on the optional `SavedSession` fields, so an old or hand-edited entry does not quarantine the whole file.
- `load` strips a UTF-8 BOM.
- A drive root stays `C:\`.
- A blank rename clears the name.
- Load and save errors name the file.
- A failed save removes its temp file.
- Docs state that only the helper writes the file.

- [ ] **Step 6: Stage**

```bash
git add crates/deck-core/src/lib.rs crates/deck-core/src/store.rs
```

---

### Task 7: Pipe framing (`deck-proto`)

**Files:**
- Modify: `crates/deck-proto/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Replace `crates/deck-proto/src/lib.rs` with:
```rust
//! Framing for the pipe between the window, the helper and the hook command
//! (spec §3.4): 1 byte type, 4 byte little-endian length, then the payload.
//! Terminal frames start their payload with a 4 byte session index.

#[cfg(test)]
mod tests {
    use super::*;

    fn all_frames() -> Vec<Frame> {
        vec![
            Frame::Control(br#"{"type":"get_state"}"#.to_vec()),
            Frame::Output { session: 7, data: b"\x1b[32mhello\x1b[0m".to_vec() },
            Frame::Input { session: 0, data: b"\r".to_vec() },
            Frame::Control(Vec::new()),
        ]
    }

    #[test]
    fn frames_round_trip() {
        for frame in all_frames() {
            let mut d = FrameDecoder::new();
            d.push(&encode(&frame));
            assert_eq!(d.next_frame().unwrap(), Some(frame));
            assert_eq!(d.next_frame().unwrap(), None);
        }
    }

    #[test]
    fn header_is_type_then_little_endian_length() {
        assert_eq!(encode(&Frame::Control(b"{}".to_vec())), vec![1, 2, 0, 0, 0, b'{', b'}']);
        assert_eq!(
            encode(&Frame::Output { session: 1, data: b"A".to_vec() }),
            vec![2, 5, 0, 0, 0, 1, 0, 0, 0, b'A']
        );
    }

    #[test]
    fn decodes_when_bytes_arrive_one_at_a_time() {
        let bytes: Vec<u8> = all_frames().iter().flat_map(encode).collect();
        let mut d = FrameDecoder::new();
        let mut got = Vec::new();
        for b in bytes {
            d.push(&[b]);
            while let Some(f) = d.next_frame().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got, all_frames());
    }

    #[test]
    fn rejects_bad_frames() {
        let mut d = FrameDecoder::new();
        d.push(&[9, 0, 0, 0, 0]);
        assert_eq!(d.next_frame(), Err(FrameError::UnknownType(9)));

        let mut d = FrameDecoder::new();
        d.push(&[1]);
        d.push(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
        assert_eq!(d.next_frame(), Err(FrameError::TooLarge(MAX_PAYLOAD + 1)));

        let mut d = FrameDecoder::new();
        d.push(&[2, 2, 0, 0, 0, 1, 0]);
        assert_eq!(d.next_frame(), Err(FrameError::MissingSession));
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-proto`
Expected: compile errors: `Frame`, `encode`, `FrameDecoder`, `FrameError`, `MAX_PAYLOAD` not found.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` block in `crates/deck-proto/src/lib.rs`:
```rust
/// Largest payload accepted. Anything bigger means a broken or hostile peer.
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

const TYPE_CONTROL: u8 = 1;
const TYPE_OUTPUT: u8 = 2;
const TYPE_INPUT: u8 = 3;
const HEADER: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A JSON control message.
    Control(Vec<u8>),
    /// Terminal output of one session, helper → window.
    Output { session: u32, data: Vec<u8> },
    /// Keyboard input for one session, window → helper.
    Input { session: u32, data: Vec<u8> },
}

/// After any error the stream is out of sync: drop the connection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("unknown frame type {0}")]
    UnknownType(u8),
    #[error("frame payload of {0} bytes is over the limit")]
    TooLarge(usize),
    #[error("terminal frame is shorter than its 4 byte session index")]
    MissingSession,
}

pub fn encode(frame: &Frame) -> Vec<u8> {
    let (kind, payload) = match frame {
        Frame::Control(json) => (TYPE_CONTROL, json.clone()),
        Frame::Output { session, data } => (TYPE_OUTPUT, with_session(*session, data)),
        Frame::Input { session, data } => (TYPE_INPUT, with_session(*session, data)),
    };
    debug_assert!(payload.len() <= MAX_PAYLOAD);
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Collects bytes from a stream and hands out complete frames.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete frame, or `None` until more bytes arrive.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.buf.len() < HEADER {
            return Ok(None);
        }
        let kind = self.buf[0];
        let len = u32::from_le_bytes([self.buf[1], self.buf[2], self.buf[3], self.buf[4]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(FrameError::TooLarge(len));
        }
        if !matches!(kind, TYPE_CONTROL | TYPE_OUTPUT | TYPE_INPUT) {
            return Err(FrameError::UnknownType(kind));
        }
        if self.buf.len() < HEADER + len {
            return Ok(None);
        }
        let payload = self.buf[HEADER..HEADER + len].to_vec();
        self.buf.drain(..HEADER + len);
        let frame = match kind {
            TYPE_CONTROL => Frame::Control(payload),
            TYPE_OUTPUT => {
                let (session, data) = split_session(payload)?;
                Frame::Output { session, data }
            }
            _ => {
                let (session, data) = split_session(payload)?;
                Frame::Input { session, data }
            }
        };
        Ok(Some(frame))
    }
}

fn with_session(session: u32, data: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + data.len());
    p.extend_from_slice(&session.to_le_bytes());
    p.extend_from_slice(data);
    p
}

fn split_session(mut payload: Vec<u8>) -> Result<(u32, Vec<u8>), FrameError> {
    if payload.len() < 4 {
        return Err(FrameError::MissingSession);
    }
    let session = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    payload.drain(..4);
    Ok((session, payload))
}
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test -p deck-proto`
Expected: `test result: ok. 4 passed`.

- [ ] **Step 5: Stage**

```bash
git add crates/deck-proto/src/lib.rs
```

---

### Task 8: Whole-workspace check

- [ ] **Step 1: Format**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo fmt --all && git diff --stat`
Expected: formatting applied; only whitespace changes, if any.

- [ ] **Step 2: Lint**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings. Fix any warning in the file it names, then rerun.

- [ ] **Step 3: All tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --workspace`
Expected: deck-core 59 passed (hook 13, state 18, git 6, claude_dir 6, store 16), deck-proto 4 passed, 0 failed.

The opus safety review of Tasks 2–3 changed the code; spec §4.1 and §4.2 now match it:
- `Stop { background_work }` keeps the session Working.
- Only SessionStart startup or resume sets Idle.
- `idle_prompt` and SessionEnd change nothing; only process exit means Asleep.
- Informational notifications are ignored.
- `<task-notification>` prompts don't name sessions.

- [ ] **Step 4: Stage**

```bash
git add -A crates/ Cargo.toml Cargo.lock rust-toolchain.toml
git status --short
```
Expected: only staged (`A`/`M`) entries, and no `.env`, `target/` or `spike/hooks.log`.

---

## What comes next (separate plans)

- **Plan 2, the helper (`deck-host`):** first the vt100 snapshot check against real Claude (spec §10). Then the pipe server with a current-user ACL, ConPTY sessions, the `hook` subcommand and hooks.json, the auto-sleep timer, wake/sleep/close, the git runner, memory reporting, the control-message types, and integration tests with `tests/fake-claude`.
- **Notes for Plan 2 from the final review (2026-10-19):**
  - Ids:
    - Add `#[serde(default)] conversation_id: String` to `SavedSession`. On load, fill it from `id` when it is empty.
    - Add `set_conversation(deck_id, conv_id)` and update the id only on `SessionStart`, including `clear`.
    - "Resume past session" always gets a fresh deck id.
    - `list_past_sessions` excludes the saved sessions' *conversation* ids.
  - Git: strip the subfolder prefix (spec §6). Add a pure `git::strip_prefix` with tests.
  - Saving:
    - Call `Settings::normalize()` on `set_settings` too.
    - If `load` returns `Err`, never save defaults over the file.
    - Have one owner for saves, debounce `touch` saves, and never fsync inside the hook-ack path.
    - Fill an empty `Project.name` from the path on load.
  - Run `list_past_sessions` and `save` on a blocking thread.
  - Use a monotonic clock for `now_ms`.
  - The control messages define the frame session index ↔ deck id mapping (`attach` returns the index).
  - Consider parsing in the hook subcommand and forwarding a small event: PostToolUse payloads can be MB-sized.
  - Spike items still open (`spike/FINDINGS.md` §10).
- **Plan 3, the window:** Tauri 2 + Vue + Tailwind v4 + xterm.js UI (layout A, menus, Needs you highlighting, taskbar flash, close dialog, Changes tab, settings), the app icon chosen in chat, and the NSIS installer.
