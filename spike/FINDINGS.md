# Spike findings: Claude Code hooks (2026-10-19)

Source: `spike/hooks.log` (38 lines) and the transcripts in
`~/.claude/projects/C--tmp-deck-spike/`. Claude Code 2.1.303.
Times are local (+03:00). The transcript uses UTC (16:44 UTC = 19:44 local).

## 1. Ids
- `CLAUDE_DECK_SESSION` reaches the hook: lines show `deck=spike-1` and `deck=spike-2`.
- `session_id` equals the `--session-id` value: `5c92f437-e16b-4d1e-8280-6bd55bde0865`.
- After `--resume` it is unchanged (same id on the resume SessionStart and later hooks).
- `transcript_path` is `...\<session_id>.jsonl` and `cwd` is `C:\tmp\deck-spike`.
- Interactive session spike-2 kept one `session_id` (`8a7a9f19-...`) for the whole run.

## 2. SessionStart
- Print run: `"source":"startup"`, then `"source":"resume"`.
- Interactive run: `"source":"startup"` with an extra `"model":"claude-opus-5-5"`.
- Resume adds: `seconds_since_last_response` (3), `context_tokens` (36441),
  `prompt_cache_likely_expired` (false), `estimated_cache_write_usd` (0.2915).
- Not seen: `clear`, `compact`.

## 3. Permission prompt
Notification payload (spike-2, 19:43:18):
`{"hook_event_name":"Notification","message":"Claude needs your permission","notification_type":"permission_prompt"}`
- It has no `permission_mode` and no `effort`.
- Two prompts came in one turn:
  - AskUserQuestion: PreToolUse 19:43:12, Notification 19:43:18, PostToolUse 19:43:28.
  - Write: PreToolUse 19:43:30, Notification 19:43:36, PostToolUse 19:43:48.
- Order: PreToolUse, then Notification, then (after approval) PostToolUse, then Stop.
- PreToolUse fires before the prompt, not after approval. No hook marks the approval itself.
- Earlier, in `permission_mode":"auto"`, Write and Bash ran with no Notification.

## 4. Background work
- Yes. Bash input had `"run_in_background":true` and PostToolUse returned `"backgroundTaskId":"b3398otxz"`.
- Stop fired while the loop ran (19:42:12 and 19:43:59):
  `"background_tasks":[{"id":"bd4s07km9","type":"shell","status":"running","description":"Print numbers 1 to 30, one per second","command":"for i in $(seq 1 30); do echo $i; sleep 1; done"}],"session_crons":[]`
- Normal Stop (nothing running): `"background_tasks":[],"session_crons":[]`.
- On finish, a UserPromptSubmit arrived with a prompt starting
  `<task-notification>\n<task-id>bd4s07km9</task-id> ... <status>completed</status>
  <summary>Background command "..." completed (exit code 0)</summary>`.
- Loop 1: notification 19:42:41, came mid-turn and reused the user prompt's `prompt_id`; Stop followed at 19:42:46 with empty arrays.
- Loop 2: notification 19:44:29, new `prompt_id` `304f39df-...`. No Stop followed (see Esc).

## 5. Esc
- Transcript: `[Request interrupted by user]` at 16:44:32 UTC = 19:44:32 local.
- It hit 3 seconds after the task-notification prompt (19:44:29), while Claude was answering it.
- Hooks in the next two minutes: none at all. Next line is SessionEnd at 19:46:25.
- Stop did not fire. No `idle_prompt` Notification came.
- So after Esc the log is silent: the deck cannot see an interrupt through hooks.

## 6. Idle notification
- Not captured. No `idle_prompt` anywhere in the log.
- Note the Esc left the turn unfinished, so the idle timer may not apply.
  Needs a test after a finished turn (section 10).

## 7. Ctrl+C twice
- SessionEnd fired: `"reason":"prompt_input_exit"` at 19:46:25.56 (spike-2).
- Delay after the keypress: not captured. The log has no keypress time, and the transcript only has cost-state entries after Esc.
- Print runs also fire SessionEnd after each run, with `"reason":"other"`.

## 8. Gotchas found
- Hook commands run through a POSIX shell on Windows, so backslash paths in `hooks.json` were stripped. Forward slashes fixed it.
- PowerShell 5.1 `Add-Content -Encoding utf8` wrote a BOM (`EF BB BF`) at the start of the log. Strip it when parsing.
- Hooks append raw JSON; newlines in `prompt` were flattened by the script, so log lines are not exact copies.

## 9. Effect on the code (already applied after review)
- Stop carries `background_work` (either `background_tasks` or `session_crons` non-empty), and that keeps the session Working.
- Only SessionStart `startup` or `resume` sets Idle.
- `idle_prompt` and SessionEnd change nothing.
- Only process exit means Asleep.
- Informational notification types are ignored. `permission_prompt` and any unknown type mean Needs you; `idle_prompt` changes nothing.
- `<task-notification>` prompts don't name sessions.

## 10. Still to test by hand (Plan 2 spike)
- 70 seconds idle after a finished turn, to capture an `idle_prompt` payload.
- `/clear` and `/compact`: which hooks fire, and whether `session_id` changes.
- Esc while a permission prompt is showing.
- How fast Ctrl+C twice exits.
