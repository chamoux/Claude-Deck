# Spike 2: real Claude through our own ConPTY code (2026-10-20)

Claude Code **2.1.304** ran under `deck_host::pty` + `screen` + `dsr` in `C:\tmp\deck-spike2`, 120×40, `--permission-mode default`, with hooks from `spike/hooks.json`. Program: `spike/pty-claude/`. Log: `spike/hooks.log` (git-ignored); the first spike's log is in `spike/hooks-1.log`.

## Answers

0. **Startup**
   - **Trust prompt.** A new folder shows a trust prompt whose default is **"No, exit"**: plain Enter quits Claude, while Down then Enter trusts the folder. SessionStart followed about 3 s later.
   - **Inherited env.** The helper ran from inside Claude Code and inherited `CLAUDE_CODE_CHILD_SESSION`. Claude then warned "Transcript saving is off". The helper must remove that variable.
   - **Session id.** `--session-id` was honoured: SessionStart carried exactly our id, plus `"source":"startup"` and `"model"`.
1. **Cursor query.** Exactly one `ESC[6n` arrived, 42 ms after spawn. Answering it kept Claude alive.
   - **Alternate screen.** Claude switches to it (`ESC[?1049h`) at about byte 1512 of 3835 startup bytes and stays there for its whole life.
2. **Turn and output volume.**
   - **Turn.** Prompt "reply with the single word ok": the turn took about 3 s. Stop came with `"background_tasks":[],"session_crons":[]`.
   - **Startup output.** 3835 bytes.
   - **Turn output.** 2376 bytes.
   - **Idle output.** About 21 bytes/s, because the status line keeps repainting. Output volume cannot tell Working from Idle.
3. **Snapshot.** A 1673-byte snapshot fed into a fresh 40×120 `Screen` gave identical contents, the same cursor (36,2) and the same alternate-screen flag.
4. **idle_prompt.** It arrived 60.3 s after Stop: `{"hook_event_name":"Notification","message":"Claude is waiting for your input","notification_type":"idle_prompt"}`.
5. **`/clear`.**
   - **Hooks.** SessionEnd `"reason":"clear"`, then 0.3 s later SessionStart `"source":"clear"` with a **new** `session_id`. Same process; no UserPromptSubmit or Stop.
6. **Permission prompt, then Esc.**
   - **Before the prompt.** Hooks were UserPromptSubmit, then PreToolUse (Write), then Notification `"notification_type":"permission_prompt","message":"Claude needs your permission"`.
   - **Lag.** The Notification came about **6 s after** the dialog was on screen.
   - **After Esc.** **No hook at all** in the next 20 s: no PostToolUse and no Stop. The screen said "User rejected write to x.txt" and Claude was idle at its prompt.
7. **Ctrl+C.**
   - **First `\x03`.** The screen showed "Press Ctrl-C again to exit".
   - **Second `\x03`** (300 ms later). Claude exited by itself 624 ms after it, with code 0.
   - **SessionEnd.** It arrived with `"reason":"prompt_input_exit"`.

## Changes made because of this spike

| Finding | Change |
|---|---|
| Esc on a permission prompt sends no hook | Typing into Needs you moves it to Working (deck-core `on_input`). `idle_prompt` about 60 s later now moves Working to Idle when there is no background work. It never clears Needs you. Spec §4.2 updated. |
| Inherited `CLAUDE_CODE_CHILD_SESSION` turns transcripts off | `SpawnSpec.env_remove` added. Core removes `CLAUDE_CODE_CHILD_SESSION` and `CLAUDECODE` for sessions and shells. |
| Claude lives in the alternate screen | Snapshot handles it (confirmed). The window's xterm must not rely on main-screen scrollback for Claude. |
| `\x03`, 300 ms, `\x03` exits in about 0.6 s | Sleep sequence kept. The 5 s kill stays only as a safety net. |
| Trust prompt default is "No, exit" | No automatic answer: trusting a folder is the user's decision. A new session stays Working (never sleeps) until the user answers in the window. |
| Permission Notification lags about 6 s | Accepted for v1. Plan 3 may also highlight from screen text ("Do you want to"). |
| Version is 2.1.304 | Noted. Payloads are unchanged from 2.1.303. |

# Spike 3 (2026-10-20, Claude 2.1.304)

- **A. Untouched session (no prompt):** no transcript is written (the project folder does not even exist).
  - `claude --resume X` exits with code 1 after about 1.8 s, printing "No conversation found with session ID: X". It sends no SessionStart.
  - `claude --session-id X` (same X) starts fresh with no error.
  - After any turn, `--session-id X` exits with code 1 after about 0.2 s: "Error: Session ID X is already in use."
  - **Change:** `SavedSession.has_conversation` decides `--resume` vs `--session-id`. If a spawn exits by itself before SessionStart, it is retried once with the other flag.
- **B. Permission answered about 0.1 s after the dialog appeared:** no Notification fired at all. No late Notification after PostToolUse/Stop in 15 s. No change.
- **C. `/clear`, then exit without typing:** the new id Y has a transcript (21 KB), and `--resume Y` works. `has_conversation` becomes true on SessionStart `clear`.
- Other notes:
  - **Trust prompt:** answering it needs a short wait before Down, then Down until "❯ Yes" is shown.
  - **Permission mode:** spawns without `--permission-mode` inherit the user's default (auto mode on this machine). Claude Deck passes no mode, so the user's own setting applies.
