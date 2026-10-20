use super::*;
use deck_core::state::SessionState;
use deck_core::store::{Settings, ShellPanel};
use deck_proto::messages::decode_event;
use tempfile::TempDir;

const W: ClientId = 1;
const W2: ClientId = 2;
const HOOK: ClientId = 99;

fn config() -> CoreConfig {
    CoreConfig {
        claude_exe: PathBuf::from("claude.exe"),
        hooks_file: PathBuf::from("C:/hooks.json"),
        shell: (PathBuf::from("pwsh.exe"), vec!["-NoLogo".to_string()]),
        ctrl_c_gap_ms: 300,
        sleep_grace_ms: 5000,
        idle_exit_ms: 10_000,
        memory_every_ms: 5000,
        pipe_name: Some(r"\\.\pipe\claude-deck-test".to_string()),
    }
}

struct Env {
    dir: TempDir,
    core: Core,
    pid: String,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let mut state = SavedState::default();
    let pid = state.add_project(dir.path().to_str().unwrap()).id.clone();
    Env {
        dir,
        core: Core::new(state, config()),
        pid,
    }
}

fn events(effects: &[Effect]) -> Vec<Event> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send {
                frame: Frame::Control(b),
                ..
            }
            | Effect::Broadcast {
                frame: Frame::Control(b),
            } => Some(decode_event(b).unwrap()),
            _ => None,
        })
        .collect()
}

fn errors(effects: &[Effect]) -> Vec<String> {
    events(effects)
        .into_iter()
        .filter_map(|e| {
            if let Event::Error { message } = e {
                Some(message)
            } else {
                None
            }
        })
        .collect()
}

fn confirms(effects: &[Effect]) -> Vec<(ConfirmAction, String)> {
    events(effects)
        .into_iter()
        .filter_map(|e| match e {
            Event::NeedsConfirm { action, message } => Some((action, message)),
            _ => None,
        })
        .collect()
}

fn changed_sessions(effects: &[Effect]) -> Vec<SessionView> {
    events(effects)
        .into_iter()
        .filter_map(|e| match e {
            Event::SessionChanged { session } => Some(session),
            _ => None,
        })
        .collect()
}

fn spawns(effects: &[Effect]) -> Vec<(&Target, &SpawnSpec)> {
    effects
        .iter()
        .filter_map(|e| {
            if let Effect::Spawn { target, spec } = e {
                Some((target, spec))
            } else {
                None
            }
        })
        .collect()
}

fn writes<'a>(effects: &'a [Effect], t: &Target) -> Vec<&'a [u8]> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Write { target, bytes } if target == t => Some(bytes.as_slice()),
            _ => None,
        })
        .collect()
}

fn has(effects: &[Effect], f: impl Fn(&Effect) -> bool) -> bool {
    effects.iter().any(f)
}

fn count_kills(effects: &[Effect], t: &Target) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Kill { target } if target == t))
        .count()
}

fn saves(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Save { .. }))
        .count()
}

fn st(id: &str) -> Target {
    Target::Session {
        deck_id: id.to_string(),
    }
}

fn sh(id: &str) -> Target {
    Target::Shell {
        project_id: id.to_string(),
    }
}

fn req(e: &mut Env, client: ClientId, r: Request, now: i64) -> Vec<Effect> {
    e.core.handle(Input::Request(client, r), now)
}

fn new_session(e: &mut Env, now: i64) -> String {
    let pid = e.pid.clone();
    req(e, W, Request::NewSession { project_id: pid }, now);
    e.core.state().sessions.last().unwrap().id.clone()
}

fn hook(e: &mut Env, deck: &str, json: String, now: i64) -> Vec<Effect> {
    req(
        e,
        HOOK,
        Request::HookEvent {
            deck_id: deck.to_string(),
            payload: json,
        },
        now,
    )
}

fn start_json(conv: &str, source: &str) -> String {
    format!(r#"{{"hook_event_name":"SessionStart","session_id":"{conv}","source":"{source}"}}"#)
}

fn stop_json(conv: &str) -> String {
    format!(r#"{{"hook_event_name":"Stop","session_id":"{conv}"}}"#)
}

fn prompt_json(conv: &str, prompt: &str) -> String {
    format!(r#"{{"hook_event_name":"UserPromptSubmit","session_id":"{conv}","prompt":"{prompt}"}}"#)
}

fn ask_json(conv: &str) -> String {
    format!(
        r#"{{"hook_event_name":"Notification","session_id":"{conv}","notification_type":"permission_prompt","message":"ok?"}}"#
    )
}

/// A new session that has reported Idle at `now`.
fn idle_session(e: &mut Env, now: i64) -> String {
    let id = new_session(e, now);
    hook(e, &id, start_json(&id, "startup"), now);
    id
}

/// An Idle session whose conversation Claude has saved (one prompt was sent).
fn used_session(e: &mut Env, now: i64) -> String {
    let id = idle_session(e, now);
    hook(e, &id, prompt_json(&id, "hello"), now);
    hook(e, &id, stop_json(&id), now);
    id
}

/// A used session whose process has exited.
fn asleep_used_session(e: &mut Env) -> String {
    let id = used_session(e, 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    id
}

fn wake(e: &mut Env, id: &str, now: i64) -> Vec<Effect> {
    req(
        e,
        W,
        Request::Wake {
            deck_id: id.to_string(),
        },
        now,
    )
}

fn attach(e: &mut Env, client: ClientId, target: Target, rows: u16, cols: u16) -> Vec<Effect> {
    req(e, client, Request::Attach { target, rows, cols }, 0)
}

fn sleep(e: &mut Env, id: &str, force: bool, now: i64) -> Vec<Effect> {
    let deck_id = id.to_string();
    req(e, W, Request::Sleep { deck_id, force }, now)
}

fn get_state(e: &mut Env) -> StateView {
    let fx = req(e, W, Request::GetState, 0);
    match events(&fx).into_iter().next().unwrap() {
        Event::State { state } => state,
        other => panic!("not a state: {other:?}"),
    }
}

fn session_view(e: &mut Env, id: &str) -> SessionView {
    get_state(e)
        .sessions
        .into_iter()
        .find(|s| s.deck_id == id)
        .unwrap()
}

// ---- 1. spawn ---------------------------------------------------------

#[test]
fn new_session_spawns_claude_with_session_id_settings_and_env() {
    let mut e = env();
    let pid = e.pid.clone();
    let fx = req(&mut e, W, Request::NewSession { project_id: pid }, 5);
    let id = e.core.state().sessions[0].id.clone();
    let sp = spawns(&fx);
    assert_eq!(sp.len(), 1);
    let (target, spec) = sp[0];
    assert_eq!(target, &st(&id));
    assert_eq!(spec.program, PathBuf::from("claude.exe"));
    assert_eq!(
        spec.args,
        ["--session-id", id.as_str(), "--settings", "C:/hooks.json"]
    );
    assert_eq!(spec.cwd, e.dir.path());
    assert!(spec.kill_tree, "sessions end with their whole tree");
    assert!(spec
        .env
        .contains(&("CLAUDE_DECK_SESSION".into(), id.clone())));
    assert!(spec.env.contains(&(
        "CLAUDE_DECK_PIPE".into(),
        r"\\.\pipe\claude-deck-test".into()
    )));
    assert!(spec.env.contains(&("TERM".into(), "xterm-256color".into())));
    assert!(spec.env.contains(&("COLORTERM".into(), "truecolor".into())));
    assert_eq!((spec.rows, spec.cols), (40, 120));
    assert_eq!(spec.env_remove, ["CLAUDE_CODE_CHILD_SESSION", "CLAUDECODE"]);
    assert_eq!(saves(&fx), 1);
    assert!(!e.core.state().session(&id).unwrap().has_conversation);
    let changed = events(&fx).into_iter().find_map(|ev| match ev {
        Event::SessionChanged { session } => Some(session),
        _ => None,
    });
    let changed = changed.expect("SessionChanged broadcast");
    assert_eq!(changed.deck_id, id);
    assert_eq!(changed.state, SessionState::Working);
}

#[test]
fn wake_resumes_the_saved_conversation() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    // /clear writes the new conversation's transcript at once (spike 3).
    hook(&mut e, &id, start_json("conv-2", "clear"), 1);
    e.core.handle(Input::PtyExited { target: st(&id) }, 2);
    let fx = req(
        &mut e,
        W,
        Request::Wake {
            deck_id: id.clone(),
        },
        3,
    );
    let sp = spawns(&fx);
    assert_eq!(
        sp[0].1.args,
        ["--resume", "conv-2", "--settings", "C:/hooks.json"]
    );
    assert!(sp[0]
        .1
        .env
        .contains(&("CLAUDE_DECK_SESSION".into(), id.clone())));
    assert_eq!(session_view(&mut e, &id).state, SessionState::Working);
}

#[test]
fn a_conversation_without_a_transcript_is_never_resumed() {
    let mut e = env();
    // A new session that never had a turn: Claude wrote no transcript.
    let id = idle_session(&mut e, 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    let fx = wake(&mut e, &id, 2);
    assert_eq!(spawns(&fx)[0].1.args[..2], ["--session-id", id.as_str()]);
    // The first prompt makes one: from then on wakes resume.
    hook(&mut e, &id, start_json(&id, "startup"), 3);
    let fx = hook(&mut e, &id, prompt_json(&id, "go"), 4);
    assert_eq!(saves(&fx), 1);
    e.core.handle(Input::PtyExited { target: st(&id) }, 5);
    let fx = wake(&mut e, &id, 6);
    assert_eq!(spawns(&fx)[0].1.args[..2], ["--resume", id.as_str()]);
    // /resume inside Claude switches to another saved conversation.
    hook(&mut e, &id, start_json(&id, "resume"), 7);
    hook(&mut e, &id, start_json("conv-old", "resume"), 8);
    e.core.handle(Input::PtyExited { target: st(&id) }, 9);
    let fx = wake(&mut e, &id, 10);
    assert_eq!(spawns(&fx)[0].1.args[..2], ["--resume", "conv-old"]);
}

#[test]
fn clear_writes_a_transcript_so_the_new_conversation_is_resumed() {
    let mut e = env();
    let id = idle_session(&mut e, 0); // no turn yet: false
    let fx = hook(&mut e, &id, start_json("conv-new", "clear"), 1);
    assert_eq!(saves(&fx), 1);
    let saved = e.core.state().session(&id).unwrap();
    assert_eq!(
        (saved.conversation_id.as_str(), saved.has_conversation),
        ("conv-new", true)
    );
    e.core.handle(Input::PtyExited { target: st(&id) }, 2);
    let fx = wake(&mut e, &id, 3);
    assert_eq!(spawns(&fx)[0].1.args[..2], ["--resume", "conv-new"]);
}

#[test]
fn any_tool_use_or_stop_also_marks_the_transcript() {
    let hooks: [fn(&str) -> String; 3] = [
        |id| format!(r#"{{"hook_event_name":"PreToolUse","session_id":"{id}"}}"#),
        |id| format!(r#"{{"hook_event_name":"PostToolUse","session_id":"{id}"}}"#),
        stop_json,
    ];
    for json in hooks {
        let mut e = env();
        let id = idle_session(&mut e, 0);
        let fx = hook(&mut e, &id, json(&id), 1);
        assert_eq!(saves(&fx), 1);
        assert!(e.core.state().session(&id).unwrap().has_conversation);
    }
}

/// Wakes `id` and lets that Claude quit by itself before any hook.
fn wake_and_quit(e: &mut Env, id: &str, now: i64) -> (Vec<Effect>, Vec<Effect>) {
    let woke = wake(e, id, now);
    let quit = e
        .core
        .handle(Input::PtyExited { target: st(id) }, now + 200);
    (woke, quit)
}

#[test]
fn a_failed_resume_is_retried_once_as_a_fresh_start() {
    let mut e = env();
    let id = asleep_used_session(&mut e);
    // Our guess was wrong: no transcript, so `--resume` quits at once.
    let (woke, quit) = wake_and_quit(&mut e, &id, 2);
    assert_eq!(spawns(&woke)[0].1.args[..2], ["--resume", id.as_str()]);
    let retry = spawns(&quit);
    assert_eq!(retry.len(), 1);
    assert_eq!(retry[0].1.args[..2], ["--session-id", id.as_str()]);
    // The window never sees it fall asleep in between, and no error.
    assert!(changed_sessions(&quit)
        .iter()
        .all(|s| s.state == SessionState::Working && s.error.is_none()));
    // It starts fresh; the next wake goes straight to --session-id.
    hook(&mut e, &id, start_json(&id, "startup"), 300);
    assert!(!e.core.state().session(&id).unwrap().has_conversation);
    e.core.handle(Input::PtyExited { target: st(&id) }, 400);
    assert_eq!(session_view(&mut e, &id).error, None);
    let fx = wake(&mut e, &id, 500);
    assert_eq!(spawns(&fx)[0].1.args[..2], ["--session-id", id.as_str()]);
}

#[test]
fn a_failed_fresh_start_is_retried_once_as_a_resume() {
    let mut e = env();
    // has_conversation says no, but a transcript exists ("Session ID … is already in use").
    let id = idle_session(&mut e, 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    let (woke, quit) = wake_and_quit(&mut e, &id, 2);
    assert_eq!(spawns(&woke)[0].1.args[..2], ["--session-id", id.as_str()]);
    assert_eq!(spawns(&quit)[0].1.args[..2], ["--resume", id.as_str()]);
    hook(&mut e, &id, start_json(&id, "resume"), 300);
    assert!(e.core.state().session(&id).unwrap().has_conversation);
    assert_eq!(session_view(&mut e, &id).error, None);
}

#[test]
fn when_the_retry_also_quits_early_it_cannot_resume() {
    let mut e = env();
    let id = asleep_used_session(&mut e);
    let (_, quit) = wake_and_quit(&mut e, &id, 2);
    assert_eq!(spawns(&quit).len(), 1);
    let fx = e.core.handle(Input::PtyExited { target: st(&id) }, 600);
    assert!(spawns(&fx).is_empty(), "only one retry");
    let changed = changed_sessions(&fx);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].state, SessionState::Asleep);
    assert_eq!(changed[0].error.as_deref(), Some("Can't resume"));
    // A later wake tries again (both flags) and clears the error.
    let fx = wake(&mut e, &id, 700);
    assert_eq!(spawns(&fx).len(), 1);
    assert_eq!(session_view(&mut e, &id).error, None);
    let quit = e.core.handle(Input::PtyExited { target: st(&id) }, 800);
    assert_eq!(spawns(&quit).len(), 1, "a new wake gets its own retry");
}

#[test]
fn a_new_session_that_quits_at_once_is_retried_too() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    let fx = e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    assert_eq!(spawns(&fx)[0].1.args[..2], ["--resume", id.as_str()]);
    e.core.handle(Input::PtyExited { target: st(&id) }, 2);
    assert_eq!(
        session_view(&mut e, &id).error.as_deref(),
        Some("Can't resume")
    );
}

#[test]
fn claude_path_setting_overrides_the_configured_exe() {
    let mut e = env();
    let s = Settings {
        claude_path: Some("D:/tools/claude.exe".into()),
        ..Settings::default()
    };
    req(&mut e, W, Request::SetSettings { settings: s }, 0);
    let pid = e.pid.clone();
    let fx = req(&mut e, W, Request::NewSession { project_id: pid }, 1);
    assert_eq!(
        spawns(&fx)[0].1.program,
        PathBuf::from("D:/tools/claude.exe")
    );
}

#[test]
fn missing_folder_does_not_spawn_and_is_listed() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    std::fs::remove_dir_all(e.dir.path()).unwrap();
    let fx = req(
        &mut e,
        W,
        Request::Wake {
            deck_id: id.clone(),
        },
        2,
    );
    assert!(spawns(&fx).is_empty());
    assert_eq!(errors(&fx), vec!["Folder missing".to_string()]);
    let pid = e.pid.clone();
    // The window learns at once that the folder is gone (spec §8).
    let missing = |fx: &[Effect]| {
        events(fx).into_iter().find_map(|ev| match ev {
            Event::State { state } => Some(state.missing_folders),
            _ => None,
        })
    };
    assert_eq!(missing(&fx), Some(vec![pid.clone()]));
    assert_eq!(get_state(&mut e).missing_folders, vec![pid.clone()]);
    let fx = req(
        &mut e,
        W,
        Request::NewSession {
            project_id: pid.clone(),
        },
        3,
    );
    assert!(spawns(&fx).is_empty());
    assert_eq!(errors(&fx).len(), 1);
    assert_eq!(missing(&fx), Some(vec![pid.clone()]));
    let fx = req(
        &mut e,
        W,
        Request::OpenPastSession {
            project_id: pid.clone(),
            conversation_id: "old".into(),
        },
        4,
    );
    assert!(spawns(&fx).is_empty());
    assert_eq!(missing(&fx), Some(vec![pid]));
    // Nothing half-made is left in the sidebar.
    assert_eq!(e.core.state().sessions.len(), 1);
    assert_eq!(saves(&fx), 0);
}

#[test]
fn shell_uses_configured_shell_without_session_env() {
    let mut e = env();
    let pid = e.pid.clone();
    let fx = req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 30,
            cols: 90,
        },
        0,
    );
    let sp = spawns(&fx);
    assert_eq!(sp.len(), 1);
    let (t, spec) = sp[0];
    assert_eq!(t, &sh(&pid));
    assert_eq!(spec.program, PathBuf::from("pwsh.exe"));
    assert_eq!(spec.args, ["-NoLogo"]);
    assert_eq!(spec.cwd, e.dir.path());
    assert!(
        !spec.kill_tree,
        "killing a shell's tree would close GUI apps"
    );
    // Started at the window's size, not 120×40.
    assert_eq!((spec.rows, spec.cols), (30, 90));
    assert!(spec.env.contains(&("TERM".into(), "xterm-256color".into())));
    assert!(spec.env.contains(&("COLORTERM".into(), "truecolor".into())));
    assert!(!spec.env.iter().any(|(k, _)| k == "CLAUDE_DECK_SESSION"));
    assert_eq!(spec.env_remove, ["CLAUDE_CODE_CHILD_SESSION", "CLAUDECODE"]);
    assert!(events(&fx).contains(&Event::ShellChanged {
        shell: ShellView {
            project_id: pid.clone(),
            running: true
        }
    }));
    assert_eq!(
        get_state(&mut e).shells,
        vec![ShellView {
            project_id: pid,
            running: true
        }]
    );
}

fn test_shell(kind: ShellKind) -> (PathBuf, Vec<String>) {
    match kind {
        ShellKind::PowerShell => (PathBuf::from("pwsh.exe"), vec!["-NoLogo".to_string()]),
        ShellKind::GitBash => (PathBuf::from("bash.exe"), vec!["--login".to_string()]),
        ShellKind::Cmd => (PathBuf::from("cmd.exe"), Vec::new()),
    }
}

#[test]
fn shell_follows_the_shell_setting_with_a_lookup() {
    let mut e = env();
    let state = e.core.state().clone();
    e.core = Core::new(state, config()).with_shell_lookup(test_shell);
    let pid = e.pid.clone();
    for (kind, program) in [
        (ShellKind::Cmd, "cmd.exe"),
        (ShellKind::GitBash, "bash.exe"),
    ] {
        let s = Settings {
            shell: kind,
            ..Settings::default()
        };
        req(&mut e, W, Request::SetSettings { settings: s }, 0);
        let fx = attach(&mut e, W, sh(&pid), 10, 10);
        assert_eq!(spawns(&fx)[0].1.program, PathBuf::from(program));
        e.core.handle(Input::PtyExited { target: sh(&pid) }, 1);
    }
}

#[test]
fn spawn_failure_removes_the_terminal_and_reports() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    let fx = e.core.handle(
        Input::SpawnFailed {
            target: st(&id),
            error: "claude.exe not found".into(),
        },
        1,
    );
    assert_eq!(errors(&fx), vec!["claude.exe not found".to_string()]);
    let v = session_view(&mut e, &id);
    assert_eq!(v.state, SessionState::Asleep);
    assert_eq!(v.error.as_deref(), Some("claude.exe not found"));
    // Not live any more: attaching is refused.
    let fx = req(
        &mut e,
        W,
        Request::Attach {
            target: st(&id),
            rows: 10,
            cols: 10,
        },
        2,
    );
    assert_eq!(errors(&fx).len(), 1);

    let pid = e.pid.clone();
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        3,
    );
    let fx = e.core.handle(
        Input::SpawnFailed {
            target: sh(&pid),
            error: "no shell".into(),
        },
        4,
    );
    assert!(events(&fx).contains(&Event::ShellChanged {
        shell: ShellView {
            project_id: pid,
            running: false
        }
    }));
    assert!(get_state(&mut e).shells.is_empty());
}

// ---- 2. exit ----------------------------------------------------------

#[test]
fn exit_puts_the_session_to_sleep_but_keeps_it_listed() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    let fx = e.core.handle(Input::PtyExited { target: st(&id) }, 5);
    let changed: Vec<_> = events(&fx)
        .into_iter()
        .filter_map(|ev| {
            if let Event::SessionChanged { session } = ev {
                Some(session)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].state, SessionState::Asleep);
    assert_eq!(changed[0].error, None);
    assert_eq!(e.core.state().sessions.len(), 1);

    let pid = e.pid.clone();
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        6,
    );
    let fx = e.core.handle(Input::PtyExited { target: sh(&pid) }, 7);
    assert!(events(&fx).contains(&Event::ShellChanged {
        shell: ShellView {
            project_id: pid,
            running: false
        }
    }));
}

#[test]
fn woken_session_that_reported_start_exits_without_error() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    req(
        &mut e,
        W,
        Request::Wake {
            deck_id: id.clone(),
        },
        2,
    );
    hook(&mut e, &id, start_json(&id, "resume"), 3);
    e.core.handle(Input::PtyExited { target: st(&id) }, 4);
    assert_eq!(session_view(&mut e, &id).error, None);
}

#[test]
fn an_exit_we_asked_for_before_the_start_hook_is_not_an_error() {
    let mut e = env();
    let id = asleep_used_session(&mut e);
    wake(&mut e, &id, 2);
    // The user changes their mind at once: no SessionStart yet.
    sleep(&mut e, &id, true, 3);
    e.core.handle(Input::PtyExited { target: st(&id) }, 600);
    assert_eq!(session_view(&mut e, &id).error, None);
    // Same for a kill after the grace time.
    wake(&mut e, &id, 1000);
    sleep(&mut e, &id, true, 1000);
    let fx = e.core.handle(Input::Tick, 7000);
    assert_eq!(count_kills(&fx, &st(&id)), 1);
    e.core.handle(Input::PtyExited { target: st(&id) }, 7100);
    assert_eq!(session_view(&mut e, &id).error, None);
}

const TRUST_SCREEN: &[u8] = b"\x1b[?1049hDo you trust the files in this folder?\r\n\x1b[7m> No, exit\x1b[m\r\n  Yes, I trust this folder";

#[test]
fn the_trust_prompt_shows_needs_you_and_declining_it_is_not_cant_resume() {
    let mut e = env();
    let id = asleep_used_session(&mut e);
    wake(&mut e, &id, 2);
    attach(&mut e, W, st(&id), 40, 120);
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: TRUST_SCREEN.to_vec(),
        },
        3,
    );
    let changed = changed_sessions(&fx);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].state, SessionState::NeedsYou);
    // It never auto-sleeps while asking.
    let fx = e.core.handle(Input::Tick, 3 + 600 * 60_000);
    assert!(writes(&fx, &st(&id)).is_empty());
    // Repaints of the same dialog do not broadcast again.
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"\x1b[H".to_vec(),
        },
        4,
    );
    assert!(changed_sessions(&fx).is_empty());
    // Plain Enter = "No, exit": Claude quits by itself, before any hook.
    e.core.handle(
        Input::TerminalInput {
            index: 0,
            bytes: b"\r".to_vec(),
        },
        5,
    );
    e.core.handle(Input::PtyExited { target: st(&id) }, 6);
    let v = session_view(&mut e, &id);
    assert_eq!(v.state, SessionState::Asleep);
    assert_eq!(
        v.error.as_deref(),
        Some("Claude quit: the folder was not trusted")
    );
    // The conversation is fine: the next wake still resumes it.
    let fx = wake(&mut e, &id, 7);
    assert_eq!(spawns(&fx)[0].1.args[0], "--resume");
}

#[test]
fn a_new_session_that_declined_trust_starts_fresh_on_the_next_wake() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: TRUST_SCREEN.to_vec(),
        },
        1,
    );
    let fx = e.core.handle(Input::PtyExited { target: st(&id) }, 2);
    assert!(spawns(&fx).is_empty(), "declining trust is not retried");
    // No conversation was ever created, so --resume would fail.
    let fx = wake(&mut e, &id, 3);
    assert_eq!(spawns(&fx)[0].1.args[..2], ["--session-id", id.as_str()]);
}

#[test]
fn trust_text_after_the_start_hook_is_ordinary_output() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: TRUST_SCREEN.to_vec(),
        },
        1,
    );
    assert!(changed_sessions(&fx).is_empty());
    assert_eq!(session_view(&mut e, &id).state, SessionState::Idle);
}

// ---- 3. hooks ---------------------------------------------------------

#[test]
fn hooks_drive_state_name_and_conversation_and_ack_comes_last() {
    let mut e = env();
    let id = new_session(&mut e, 0);

    let fx = hook(&mut e, &id, start_json("conv-9", "startup"), 10);
    assert!(matches!(fx.last(), Some(Effect::HookAck { client: HOOK })));
    assert_eq!(
        e.core.state().session(&id).unwrap().conversation_id,
        "conv-9"
    );
    assert_eq!(saves(&fx), 1);
    let session = events(&fx)
        .into_iter()
        .find_map(|ev| {
            if let Event::SessionChanged { session } = ev {
                Some(session)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(session.state, SessionState::Idle);

    let fx = hook(&mut e, &id, prompt_json("conv-9", "fix the login bug"), 20);
    assert!(matches!(fx.last(), Some(Effect::HookAck { .. })));
    assert_eq!(
        e.core.state().session(&id).unwrap().name.as_deref(),
        Some("fix the login bug")
    );
    assert_eq!(e.core.state().session(&id).unwrap().last_active_ms, 20);
    assert_eq!(saves(&fx), 1);
    let session = events(&fx)
        .into_iter()
        .find_map(|ev| {
            if let Event::SessionChanged { session } = ev {
                Some(session)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        (session.state, session.name.as_deref()),
        (SessionState::Working, Some("fix the login bug"))
    );

    let fx = hook(&mut e, &id, ask_json("conv-9"), 30);
    assert_eq!(session_view(&mut e, &id).state, SessionState::NeedsYou);
    assert!(matches!(fx.last(), Some(Effect::HookAck { .. })));
    hook(&mut e, &id, stop_json("conv-9"), 40);
    assert_eq!(session_view(&mut e, &id).state, SessionState::Idle);
}

#[test]
fn clear_start_updates_the_conversation_without_changing_state() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    hook(&mut e, &id, prompt_json(&id, "go"), 1);
    let fx = hook(&mut e, &id, start_json("conv-new", "clear"), 2);
    assert_eq!(
        e.core.state().session(&id).unwrap().conversation_id,
        "conv-new"
    );
    assert_eq!(saves(&fx), 1);
    assert_eq!(session_view(&mut e, &id).state, SessionState::Working);
}

#[test]
fn unknown_deck_or_bad_json_only_acks() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    let fx = hook(&mut e, "nope", start_json("c", "startup"), 1);
    assert!(matches!(fx.as_slice(), [Effect::HookAck { client: HOOK }]));
    let fx = hook(&mut e, &id, "{not json".into(), 1);
    assert!(matches!(fx.as_slice(), [Effect::HookAck { client: HOOK }]));
}

#[test]
fn hook_that_changes_nothing_emits_no_save_or_session_changed() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    hook(&mut e, &id, prompt_json(&id, "go"), 1);
    let fx = hook(&mut e, &id, prompt_json(&id, "again"), 2);
    assert!(matches!(fx.as_slice(), [Effect::HookAck { .. }]));
    let tool = format!(r#"{{"hook_event_name":"PreToolUse","session_id":"{id}"}}"#);
    let fx = hook(&mut e, &id, tool, 3);
    assert!(matches!(fx.as_slice(), [Effect::HookAck { .. }]));
}

#[test]
fn hook_for_a_sleeping_session_is_ignored() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    let fx = hook(&mut e, &id, prompt_json(&id, "late"), 2);
    assert!(matches!(fx.as_slice(), [Effect::HookAck { .. }]));
    assert_eq!(session_view(&mut e, &id).state, SessionState::Asleep);
}

// ---- 4. output --------------------------------------------------------

#[test]
fn cursor_query_is_answered_with_the_cursor_where_it_was_asked() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    req(
        &mut e,
        W,
        Request::Attach {
            target: st(&id),
            rows: 24,
            cols: 80,
        },
        1,
    );
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"ab\x1b[6ncd".to_vec(),
        },
        2,
    );
    // Both the answer and the clean bytes are produced; the cursor was after "ab" when asked.
    assert_eq!(writes(&fx, &st(&id)), vec![b"\x1b[1;3R".as_slice()]);
    // The answer comes before the output is forwarded.
    let pos_reply = fx
        .iter()
        .position(|f| matches!(f, Effect::Write { .. }))
        .unwrap();
    let pos_out = fx
        .iter()
        .position(|f| matches!(f, Effect::Send { .. }))
        .unwrap();
    assert!(pos_reply < pos_out);
    let out: Vec<_> = fx
        .iter()
        .filter_map(|f| match f {
            Effect::Send {
                client,
                frame: Frame::Output { data, .. },
            } => Some((*client, data.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(out, vec![(W, b"abcd".to_vec())]);
    // Now the cursor has moved.
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"\x1b[6n".to_vec(),
        },
        3,
    );
    assert_eq!(writes(&fx, &st(&id)), vec![b"\x1b[1;5R".as_slice()]);
    // Nothing but a query: no empty Output frame.
    assert!(!has(&fx, |f| matches!(
        f,
        Effect::Send {
            frame: Frame::Output { .. },
            ..
        }
    )));
    // Several queries in one chunk: each gets the cursor at its own place.
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"\r\x1b[6nxy\x1b[6n".to_vec(),
        },
        4,
    );
    assert_eq!(
        writes(&fx, &st(&id)),
        vec![b"\x1b[1;1R".as_slice(), b"\x1b[1;3R".as_slice()]
    );
}

#[test]
fn output_goes_to_every_attached_window_only() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    req(
        &mut e,
        W,
        Request::Attach {
            target: st(&id),
            rows: 24,
            cols: 80,
        },
        1,
    );
    req(
        &mut e,
        W2,
        Request::Attach {
            target: st(&id),
            rows: 24,
            cols: 80,
        },
        1,
    );
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"hi".to_vec(),
        },
        2,
    );
    let to: Vec<ClientId> = fx
        .iter()
        .filter_map(|f| {
            if let Effect::Send {
                client,
                frame: Frame::Output { .. },
            } = f
            {
                Some(*client)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(to, vec![W, W2]);
    // Output before anyone attached, or for an unknown terminal, produces nothing.
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st("zzz"),
            bytes: b"hi".to_vec(),
        },
        3,
    );
    assert!(fx.is_empty());
}

// ---- 5. attach --------------------------------------------------------

#[test]
fn attach_replies_with_index_then_snapshot_and_applies_the_size() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"hello".to_vec(),
        },
        1,
    );
    let fx = req(
        &mut e,
        W,
        Request::Attach {
            target: st(&id),
            rows: 20,
            cols: 50,
        },
        2,
    );
    // Order: Resize, Attached, snapshot.
    let pos_resize = fx
        .iter()
        .position(|f| {
            matches!(
                f,
                Effect::Resize {
                    rows: 20,
                    cols: 50,
                    ..
                }
            )
        })
        .expect("Resize");
    assert_eq!(pos_resize, 0);
    assert_eq!(
        events(&fx),
        vec![Event::Attached {
            target: st(&id),
            index: 0
        }]
    );
    let pos_attached = fx
        .iter()
        .position(|f| {
            matches!(
                f,
                Effect::Send {
                    frame: Frame::Control(_),
                    ..
                }
            )
        })
        .unwrap();
    let pos_output = fx
        .iter()
        .position(|f| {
            matches!(
                f,
                Effect::Send {
                    frame: Frame::Output { .. },
                    ..
                }
            )
        })
        .unwrap();
    assert!(pos_attached < pos_output);
    let Effect::Send {
        frame: Frame::Output { session, data },
        ..
    } = &fx[pos_output]
    else {
        unreachable!()
    };
    assert_eq!(*session, 0);
    assert!(String::from_utf8_lossy(data).contains("hello"));
    // The headless screen took the window's size too: the cursor is clamped to 20×50.
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"\x1b[40;120H\x1b[6n".to_vec(),
        },
        3,
    );
    assert_eq!(writes(&fx, &st(&id)), vec![b"\x1b[20;50R".as_slice()]);
}

#[test]
fn indices_are_monotonic_and_stable_while_the_terminal_lives() {
    let mut e = env();
    let a = new_session(&mut e, 0);
    let b = new_session(&mut e, 0);
    let idx = |fx: &[Effect]| match events(fx).into_iter().next().unwrap() {
        Event::Attached { index, .. } => index,
        other => panic!("{other:?}"),
    };
    let fx = req(
        &mut e,
        W,
        Request::Attach {
            target: st(&a),
            rows: 10,
            cols: 10,
        },
        1,
    );
    assert_eq!(idx(&fx), 0);
    let fx = req(
        &mut e,
        W,
        Request::Attach {
            target: st(&b),
            rows: 10,
            cols: 10,
        },
        1,
    );
    assert_eq!(idx(&fx), 1);
    req(&mut e, W, Request::Detach { target: st(&a) }, 2);
    let fx = req(
        &mut e,
        W2,
        Request::Attach {
            target: st(&a),
            rows: 10,
            cols: 10,
        },
        3,
    );
    assert_eq!(idx(&fx), 0);
    // After a sleep and wake the terminal is new and gets a new index.
    e.core.handle(Input::PtyExited { target: st(&a) }, 4);
    req(&mut e, W, Request::Wake { deck_id: a.clone() }, 5);
    let fx = req(
        &mut e,
        W,
        Request::Attach {
            target: st(&a),
            rows: 10,
            cols: 10,
        },
        6,
    );
    assert_eq!(idx(&fx), 2);
}

#[test]
fn attaching_an_asleep_session_is_an_error() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    let fx = req(
        &mut e,
        W,
        Request::Attach {
            target: st(&id),
            rows: 10,
            cols: 10,
        },
        2,
    );
    assert_eq!(errors(&fx).len(), 1);
    assert!(spawns(&fx).is_empty());
}

#[test]
fn attaching_a_shell_starts_it_then_replies() {
    let mut e = env();
    let pid = e.pid.clone();
    let fx = req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 30,
            cols: 90,
        },
        0,
    );
    assert_eq!(spawns(&fx).len(), 1);
    assert!(has(&fx, |f| matches!(
        f,
        Effect::Resize {
            rows: 30,
            cols: 90,
            ..
        }
    )));
    assert!(events(&fx).contains(&Event::Attached {
        target: sh(&pid),
        index: 0
    }));
    // A second attach does not spawn again.
    let fx = req(
        &mut e,
        W2,
        Request::Attach {
            target: sh(&pid),
            rows: 30,
            cols: 90,
        },
        1,
    );
    assert!(spawns(&fx).is_empty());
}

#[test]
fn detach_stops_forwarding_to_that_client() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    req(
        &mut e,
        W,
        Request::Attach {
            target: st(&id),
            rows: 10,
            cols: 10,
        },
        1,
    );
    req(
        &mut e,
        W2,
        Request::Attach {
            target: st(&id),
            rows: 10,
            cols: 10,
        },
        1,
    );
    let fx = req(&mut e, W, Request::Detach { target: st(&id) }, 2);
    assert_eq!(events(&fx), vec![Event::Detached { target: st(&id) }]);
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"x".to_vec(),
        },
        3,
    );
    let to: Vec<ClientId> = fx
        .iter()
        .filter_map(|f| {
            if let Effect::Send {
                client,
                frame: Frame::Output { .. },
            } = f
            {
                Some(*client)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(to, vec![W2]);
    // A vanished client stops receiving too.
    e.core.handle(Input::ClientGone(W2), 4);
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"x".to_vec(),
        },
        5,
    );
    assert!(fx.is_empty());
}

#[test]
fn resize_request_resizes_the_pty() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    let fx = req(
        &mut e,
        W,
        Request::Resize {
            target: st(&id),
            rows: 50,
            cols: 160,
        },
        1,
    );
    assert!(matches!(
        fx.as_slice(),
        [Effect::Resize {
            rows: 50,
            cols: 160,
            ..
        }]
    ));
    // The headless screen follows: row 45 exists now (it did not at 40 rows).
    let fx = e.core.handle(
        Input::PtyOutput {
            target: st(&id),
            bytes: b"\x1b[45;150H\x1b[6n".to_vec(),
        },
        2,
    );
    assert_eq!(writes(&fx, &st(&id)), vec![b"\x1b[45;150R".as_slice()]);
}

// ---- 6. input ---------------------------------------------------------

#[test]
fn terminal_input_is_written_to_the_target() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    req(
        &mut e,
        W,
        Request::Attach {
            target: st(&id),
            rows: 10,
            cols: 10,
        },
        1,
    );
    let fx = e.core.handle(
        Input::TerminalInput {
            index: 0,
            bytes: b"ls\r".to_vec(),
        },
        2,
    );
    assert_eq!(writes(&fx, &st(&id)), vec![b"ls\r".as_slice()]);
    // Unknown index: dropped.
    assert!(e
        .core
        .handle(
            Input::TerminalInput {
                index: 7,
                bytes: b"x".to_vec()
            },
            3
        )
        .is_empty());
}

#[test]
fn input_answers_a_needs_you_session_and_broadcasts_the_change() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    hook(&mut e, &id, ask_json(&id), 1);
    req(
        &mut e,
        W,
        Request::Attach {
            target: st(&id),
            rows: 10,
            cols: 10,
        },
        2,
    );
    let fx = e.core.handle(
        Input::TerminalInput {
            index: 0,
            bytes: b"y".to_vec(),
        },
        3,
    );
    let session = events(&fx)
        .into_iter()
        .find_map(|ev| {
            if let Event::SessionChanged { session } = ev {
                Some(session)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(session.state, SessionState::Working);
}

#[test]
fn focus_mouse_and_query_replies_from_the_window_do_not_answer_needs_you() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    hook(&mut e, &id, ask_json(&id), 1);
    attach(&mut e, W, st(&id), 10, 10);
    let reports: [&[u8]; 8] = [
        b"\x1b[I",
        b"\x1b[O",
        b"\x1b[<64;10;5M",
        b"\x1b[<0;3;4M\x1b[<0;3;4m",
        b"\x1b[M !!",
        b"\x1b[?62;22c",
        b"\x1b[?2026;2$y",
        b"\x1b]11;rgb:0000/0000/0000\x07",
    ];
    for report in reports {
        let input = Input::TerminalInput {
            index: 0,
            bytes: report.to_vec(),
        };
        let fx = e.core.handle(input, 3);
        assert_eq!(writes(&fx, &st(&id)), vec![report], "still forwarded");
        assert!(
            changed_sessions(&fx).is_empty(),
            "{report:?} cleared Needs you"
        );
    }
    assert_eq!(session_view(&mut e, &id).state, SessionState::NeedsYou);
    // Real keys still answer: Alt+[ (a bare ESC [) and Esc are keys.
    for key in [&b"\x1b["[..], b"\x1b"] {
        hook(&mut e, &id, ask_json(&id), 4);
        let input = Input::TerminalInput {
            index: 0,
            bytes: key.to_vec(),
        };
        let fx = e.core.handle(input, 5);
        assert_eq!(
            changed_sessions(&fx)[0].state,
            SessionState::Working,
            "{key:?}"
        );
    }
}

#[test]
fn input_in_an_idle_session_restarts_the_idle_clock_silently() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    req(
        &mut e,
        W,
        Request::Attach {
            target: st(&id),
            rows: 10,
            cols: 10,
        },
        1,
    );
    let thirty = 30 * 60_000;
    let fx = e.core.handle(
        Input::TerminalInput {
            index: 0,
            bytes: b"a".to_vec(),
        },
        thirty - 1,
    );
    assert!(events(&fx).is_empty());
    let fx = e.core.handle(Input::Tick, thirty);
    assert!(writes(&fx, &st(&id)).is_empty());
    let fx = e.core.handle(Input::Tick, thirty - 1 + thirty);
    assert_eq!(writes(&fx, &st(&id)), vec![[3u8].as_slice()]);
}

// ---- 7. auto-sleep ----------------------------------------------------

#[test]
fn idle_sessions_sleep_after_the_configured_time_and_busy_ones_do_not() {
    let mut e = env();
    let idle = idle_session(&mut e, 0);
    let busy = new_session(&mut e, 0); // no hook yet: safe default Working
    let thirty = 30 * 60_000;
    let fx = e.core.handle(Input::Tick, thirty - 1);
    assert!(writes(&fx, &st(&idle)).is_empty());
    let fx = e.core.handle(Input::Tick, thirty);
    assert_eq!(writes(&fx, &st(&idle)), vec![[3u8].as_slice()]);
    assert!(writes(&fx, &st(&busy)).is_empty());
}

#[test]
fn auto_sleep_off_never_sleeps() {
    let mut e = env();
    let s = Settings {
        auto_sleep_minutes: None,
        ..Settings::default()
    };
    req(&mut e, W, Request::SetSettings { settings: s }, 0);
    let id = idle_session(&mut e, 0);
    let fx = e.core.handle(Input::Tick, 100 * 60_000 * 60);
    assert!(writes(&fx, &st(&id)).is_empty());
}

// ---- 8. sleep sequence ------------------------------------------------

#[test]
fn sleep_sends_ctrl_c_twice_then_kills() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    let t = st(&id);
    let fx = req(
        &mut e,
        W,
        Request::Sleep {
            deck_id: id.clone(),
            force: false,
        },
        1000,
    );
    assert_eq!(writes(&fx, &t), vec![[3u8].as_slice()]);
    let fx = e.core.handle(Input::Tick, 1250);
    assert!(writes(&fx, &t).is_empty(), "second Ctrl+C before the gap");
    assert_eq!(count_kills(&fx, &t), 0);
    let fx = e.core.handle(Input::Tick, 1300);
    assert_eq!(writes(&fx, &t), vec![[3u8].as_slice()]);
    let fx = e.core.handle(Input::Tick, 1550);
    assert!(writes(&fx, &t).is_empty());
    assert_eq!(count_kills(&fx, &t), 0);
    let fx = e.core.handle(Input::Tick, 5999);
    assert_eq!(count_kills(&fx, &t), 0);
    let fx = e.core.handle(Input::Tick, 6000);
    assert_eq!(count_kills(&fx, &t), 1);
    let fx = e.core.handle(Input::Tick, 6250);
    assert_eq!(count_kills(&fx, &t), 0);
}

#[test]
fn exit_during_the_sleep_cancels_the_rest() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    let t = st(&id);
    req(
        &mut e,
        W,
        Request::Sleep {
            deck_id: id.clone(),
            force: false,
        },
        1000,
    );
    e.core.handle(Input::PtyExited { target: t.clone() }, 1100);
    let fx = e.core.handle(Input::Tick, 7000);
    assert!(writes(&fx, &t).is_empty());
    assert_eq!(count_kills(&fx, &t), 0);
}

#[test]
fn sleeping_a_busy_session_asks_first_and_force_goes_ahead() {
    let mut e = env();
    let id = new_session(&mut e, 0); // Working
    let t = st(&id);
    let fx = req(
        &mut e,
        W,
        Request::Sleep {
            deck_id: id.clone(),
            force: false,
        },
        1,
    );
    assert_eq!(
        confirms(&fx),
        vec![(
            ConfirmAction::Sleep {
                deck_id: id.clone()
            },
            "Claude may be working. Stop it anyway?".to_string()
        )]
    );
    // Only to the asker, and nothing else happens.
    assert!(matches!(fx.as_slice(), [Effect::Send { client: W, .. }]));
    let fx = req(
        &mut e,
        W,
        Request::Sleep {
            deck_id: id.clone(),
            force: true,
        },
        2,
    );
    assert_eq!(writes(&fx, &t), vec![[3u8].as_slice()]);
}

#[test]
fn needs_you_also_needs_confirmation() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    hook(&mut e, &id, ask_json(&id), 1);
    let fx = sleep(&mut e, &id, false, 2);
    assert_eq!(confirms(&fx).len(), 1);
    assert!(writes(&fx, &st(&id)).is_empty());
}

#[test]
fn background_work_also_needs_confirmation() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    let stop = format!(
        r#"{{"hook_event_name":"Stop","session_id":"{id}","background_tasks":[{{"id":"1"}}]}}"#
    );
    hook(&mut e, &id, stop, 1);
    let fx = req(
        &mut e,
        W,
        Request::Sleep {
            deck_id: id.clone(),
            force: false,
        },
        2,
    );
    assert_eq!(confirms(&fx).len(), 1);
    assert!(writes(&fx, &st(&id)).is_empty());
}

#[test]
fn a_session_already_sleeping_is_not_restarted() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    let t = st(&id);
    req(
        &mut e,
        W,
        Request::Sleep {
            deck_id: id.clone(),
            force: false,
        },
        1000,
    );
    let fx = req(
        &mut e,
        W,
        Request::Sleep {
            deck_id: id.clone(),
            force: true,
        },
        1100,
    );
    assert!(fx.is_empty());
    // The timeline still follows the first request.
    let fx = e.core.handle(Input::Tick, 1300);
    assert_eq!(writes(&fx, &t).len(), 1);
}

#[test]
fn sleeping_an_asleep_session_does_nothing() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    let fx = req(
        &mut e,
        W,
        Request::Sleep {
            deck_id: id,
            force: true,
        },
        2,
    );
    assert!(fx.is_empty());
}

#[test]
fn waking_an_awake_session_never_spawns_a_second_claude() {
    let mut e = env();
    let id = used_session(&mut e, 0);
    let fx = wake(&mut e, &id, 1);
    assert!(spawns(&fx).is_empty());
    // An unrelated exit later does not start one either (no queued wake).
    let fx = e.core.handle(Input::PtyExited { target: st(&id) }, 2);
    assert!(spawns(&fx).is_empty());
}

#[test]
fn a_wake_during_a_sleep_starts_it_again_after_the_exit() {
    let mut e = env();
    let id = used_session(&mut e, 0);
    sleep(&mut e, &id, false, 1000);
    let fx = wake(&mut e, &id, 1100);
    assert!(
        spawns(&fx).is_empty(),
        "never two processes for one deck id"
    );
    let fx = e.core.handle(Input::PtyExited { target: st(&id) }, 1700);
    let sp = spawns(&fx);
    assert_eq!(sp.len(), 1);
    assert_eq!(sp[0].1.args[..2], ["--resume", id.as_str()]);
    // The window sees it go Asleep, then come back Working.
    let states: Vec<SessionState> = changed_sessions(&fx).iter().map(|s| s.state).collect();
    assert_eq!(states, vec![SessionState::Asleep, SessionState::Working]);
    // The new process gets a fresh sleep plan: no leftover Ctrl+C or kill.
    let fx = e.core.handle(Input::Tick, 7000);
    assert!(writes(&fx, &st(&id)).is_empty());
    assert_eq!(count_kills(&fx, &st(&id)), 0);
}

#[test]
fn a_wake_during_a_close_does_not_bring_it_back() {
    let mut e = env();
    let id = used_session(&mut e, 0);
    let close = Request::CloseSession {
        deck_id: id.clone(),
        force: false,
    };
    req(&mut e, W, close, 1000);
    let fx = wake(&mut e, &id, 1100);
    assert_eq!(errors(&fx), vec!["Unknown session".to_string()]);
    let fx = e.core.handle(Input::PtyExited { target: st(&id) }, 1700);
    assert!(spawns(&fx).is_empty());
}

#[test]
fn hooks_during_a_sleep_neither_cancel_it_nor_start_another() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    let t = st(&id);
    sleep(&mut e, &id, false, 1000);
    let end = format!(
        r#"{{"hook_event_name":"SessionEnd","session_id":"{id}","reason":"prompt_input_exit"}}"#
    );
    let fx = hook(&mut e, &id, end, 1100);
    assert!(matches!(fx.as_slice(), [Effect::HookAck { .. }]));
    hook(&mut e, &id, stop_json(&id), 1200); // Idle again: must not restart the sleep
    let fx = e.core.handle(Input::Tick, 1300);
    assert_eq!(writes(&fx, &t), vec![[3u8].as_slice()]);
    let fx = e.core.handle(Input::Tick, 6000);
    assert_eq!(count_kills(&fx, &t), 1);
    assert!(writes(&fx, &t).is_empty());
}

// ---- 9. close session -------------------------------------------------

#[test]
fn close_session_sleeps_removes_saves_and_tolerates_the_late_exit() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    let t = st(&id);
    let fx = req(
        &mut e,
        W,
        Request::CloseSession {
            deck_id: id.clone(),
            force: false,
        },
        10,
    );
    assert_eq!(writes(&fx, &t), vec![[3u8].as_slice()]);
    assert!(e.core.state().session(&id).is_none());
    assert_eq!(saves(&fx), 1);
    let Some(Event::State { state }) = events(&fx).into_iter().next() else {
        panic!("State broadcast")
    };
    assert!(state.sessions.is_empty());
    // The sleep sequence continues, and the exit is ignored without a broadcast.
    let fx = e.core.handle(Input::Tick, 310);
    assert_eq!(writes(&fx, &t).len(), 1);
    let fx = e.core.handle(Input::PtyExited { target: t }, 400);
    assert!(events(&fx).is_empty());
    let fx = hook(&mut e, &id, prompt_json(&id, "late"), 500);
    assert!(matches!(fx.as_slice(), [Effect::HookAck { client: HOOK }]));
}

#[test]
fn closing_a_busy_session_asks_unless_forced() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    let fx = req(
        &mut e,
        W,
        Request::CloseSession {
            deck_id: id.clone(),
            force: false,
        },
        1,
    );
    assert_eq!(
        confirms(&fx),
        vec![(
            ConfirmAction::CloseSession {
                deck_id: id.clone()
            },
            "Claude may be working. Stop it anyway?".to_string()
        )]
    );
    assert!(e.core.state().session(&id).is_some());
    let fx = req(
        &mut e,
        W,
        Request::CloseSession {
            deck_id: id.clone(),
            force: true,
        },
        2,
    );
    assert!(e.core.state().session(&id).is_none());
    assert_eq!(writes(&fx, &st(&id)).len(), 1);
}

#[test]
fn closing_an_asleep_session_just_removes_it() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 1);
    let fx = req(
        &mut e,
        W,
        Request::CloseSession {
            deck_id: id.clone(),
            force: false,
        },
        2,
    );
    assert!(e.core.state().session(&id).is_none());
    assert_eq!(saves(&fx), 1);
    assert!(!has(&fx, |f| matches!(f, Effect::Write { .. })));
}

// ---- 10. window closing -----------------------------------------------

#[test]
fn keep_running_sleeps_idle_sessions_only() {
    let mut e = env();
    let idle = idle_session(&mut e, 0);
    let busy = new_session(&mut e, 0);
    let pid = e.pid.clone();
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        0,
    );
    let fx = req(&mut e, W, Request::WindowClosing { keep_running: true }, 5);
    assert_eq!(writes(&fx, &st(&idle)).len(), 1);
    assert!(writes(&fx, &st(&busy)).is_empty());
    assert_eq!(count_kills(&fx, &sh(&pid)), 0);
}

#[test]
fn stop_and_close_sleeps_everything_and_kills_shells() {
    let mut e = env();
    let idle = idle_session(&mut e, 0);
    let busy = new_session(&mut e, 0);
    let pid = e.pid.clone();
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        0,
    );
    let fx = req(
        &mut e,
        W,
        Request::WindowClosing {
            keep_running: false,
        },
        5,
    );
    assert_eq!(writes(&fx, &st(&idle)).len(), 1);
    assert_eq!(writes(&fx, &st(&busy)).len(), 1);
    assert_eq!(count_kills(&fx, &sh(&pid)), 1);
    // The window then disconnects: no second kill.
    let fx = e.core.handle(Input::ClientGone(W), 6);
    assert_eq!(count_kills(&fx, &sh(&pid)), 0);
}

#[test]
fn last_window_leaving_kills_shells_unless_keep_running() {
    let mut e = env();
    let pid = e.pid.clone();
    e.core.handle(Input::WindowConnected(W), 0);
    e.core.handle(Input::WindowConnected(W2), 0);
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        0,
    );
    let fx = e.core.handle(Input::ClientGone(W), 1);
    assert_eq!(count_kills(&fx, &sh(&pid)), 0);
    let fx = e.core.handle(Input::ClientGone(W2), 2);
    assert_eq!(count_kills(&fx, &sh(&pid)), 1);

    // keep_running: true spares them.
    let mut e = env();
    let pid = e.pid.clone();
    e.core.handle(Input::WindowConnected(W), 0);
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        0,
    );
    req(&mut e, W, Request::WindowClosing { keep_running: true }, 1);
    let fx = e.core.handle(Input::ClientGone(W), 2);
    assert_eq!(count_kills(&fx, &sh(&pid)), 0);
    // A later window that leaves without saying so kills them.
    e.core.handle(Input::WindowConnected(W2), 3);
    let fx = e.core.handle(Input::ClientGone(W2), 4);
    assert_eq!(count_kills(&fx, &sh(&pid)), 1);
}

#[test]
fn hook_clients_leaving_do_not_count_as_windows() {
    let mut e = env();
    let pid = e.pid.clone();
    e.core.handle(Input::WindowConnected(W), 0);
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        0,
    );
    let fx = e.core.handle(Input::ClientGone(HOOK), 1);
    assert_eq!(count_kills(&fx, &sh(&pid)), 0);
}

/// A Working, a Needs you and a background-work session.
fn busy_sessions(e: &mut Env) -> Vec<String> {
    let working = new_session(e, 0);
    let needs = idle_session(e, 0);
    hook(e, &needs, ask_json(&needs), 1);
    let bg = idle_session(e, 0);
    let stop = format!(
        r#"{{"hook_event_name":"Stop","session_id":"{bg}","background_tasks":[{{"id":"1"}}]}}"#
    );
    hook(e, &bg, stop, 1);
    vec![working, needs, bg]
}

fn touched(fx: &[Effect], id: &str) -> bool {
    !writes(fx, &st(id)).is_empty() || count_kills(fx, &st(id)) > 0
}

#[test]
fn a_window_vanishing_never_stops_a_session() {
    let mut e = env();
    e.core.handle(Input::WindowConnected(W), 0);
    let mut ids = busy_sessions(&mut e);
    ids.push(idle_session(&mut e, 0));
    let fx = e.core.handle(Input::ClientGone(W), 2);
    for id in &ids {
        assert!(!touched(&fx, id), "{id} was stopped");
    }
}

#[test]
fn keep_running_spares_working_needs_you_and_background_work_for_hours() {
    let mut e = env();
    let ids = busy_sessions(&mut e);
    let mut fx = req(&mut e, W, Request::WindowClosing { keep_running: true }, 2);
    fx.extend(e.core.handle(Input::ClientGone(W), 3));
    for t in 1..=40 {
        fx.extend(e.core.handle(Input::Tick, t * 30 * 60_000));
    }
    for id in &ids {
        assert!(!touched(&fx, id), "{id} was stopped");
    }
    assert!(!has(&fx, |f| matches!(f, Effect::Exit)));
}

// ---- 11. idle exit ----------------------------------------------------

#[test]
fn shells_do_not_keep_the_helper_alive_without_a_window_or_a_session() {
    let mut e = env();
    let pid = e.pid.clone();
    e.core.handle(Input::WindowConnected(W), 0);
    attach(&mut e, W, sh(&pid), 10, 10);
    req(&mut e, W, Request::WindowClosing { keep_running: true }, 1);
    let fx = e.core.handle(Input::ClientGone(W), 1000);
    assert_eq!(
        count_kills(&fx, &sh(&pid)),
        0,
        "keep running spares the shell"
    );
    let fx = e.core.handle(Input::Tick, 10_999);
    assert_eq!(count_kills(&fx, &sh(&pid)), 0);
    // No window and no session for idle_exit_ms: the shell goes first...
    let fx = e.core.handle(Input::Tick, 11_000);
    assert_eq!(count_kills(&fx, &sh(&pid)), 1);
    assert!(!has(&fx, |f| matches!(f, Effect::Exit)));
    // ...then the helper exits once it has.
    e.core.handle(Input::PtyExited { target: sh(&pid) }, 11_100);
    let fx = e.core.handle(Input::Tick, 11_250);
    assert!(has(&fx, |f| matches!(f, Effect::Exit)));
}

#[test]
fn a_session_keeps_the_shells_and_the_helper_alive() {
    let mut e = env();
    let pid = e.pid.clone();
    e.core.handle(Input::WindowConnected(W), 0);
    let busy = idle_session(&mut e, 0);
    hook(&mut e, &busy, prompt_json(&busy, "long job"), 0); // Working
    attach(&mut e, W, sh(&pid), 10, 10);
    req(&mut e, W, Request::WindowClosing { keep_running: true }, 1);
    e.core.handle(Input::ClientGone(W), 1);
    let fx = e.core.handle(Input::Tick, 60_000);
    assert_eq!(count_kills(&fx, &sh(&pid)), 0);
    assert!(!has(&fx, |f| matches!(f, Effect::Exit)));
    // The session ends: the countdown starts from then.
    e.core
        .handle(Input::PtyExited { target: st(&busy) }, 60_000);
    let fx = e.core.handle(Input::Tick, 69_999);
    assert_eq!(count_kills(&fx, &sh(&pid)), 0);
    let fx = e.core.handle(Input::Tick, 70_000);
    assert_eq!(count_kills(&fx, &sh(&pid)), 1);
}

#[test]
fn helper_exits_once_when_nothing_is_live_and_no_window_is_connected() {
    let mut e = env();
    assert!(!has(&e.core.handle(Input::Tick, 1000), |f| matches!(
        f,
        Effect::Exit
    )));
    assert!(!has(&e.core.handle(Input::Tick, 10_999), |f| matches!(
        f,
        Effect::Exit
    )));
    assert!(has(&e.core.handle(Input::Tick, 11_000), |f| matches!(
        f,
        Effect::Exit
    )));
    assert!(!has(&e.core.handle(Input::Tick, 12_000), |f| matches!(
        f,
        Effect::Exit
    )));
}

#[test]
fn a_window_or_a_live_terminal_keeps_the_helper_running() {
    let mut e = env();
    e.core.handle(Input::WindowConnected(W), 0);
    assert!(!has(&e.core.handle(Input::Tick, 60_000), |f| matches!(
        f,
        Effect::Exit
    )));
    // Window leaves; the clock starts then.
    e.core.handle(Input::ClientGone(W), 60_000);
    assert!(!has(&e.core.handle(Input::Tick, 69_000), |f| matches!(
        f,
        Effect::Exit
    )));
    assert!(has(&e.core.handle(Input::Tick, 70_000), |f| matches!(
        f,
        Effect::Exit
    )));

    let mut e = env();
    let id = idle_session(&mut e, 0);
    let s = Settings {
        auto_sleep_minutes: None,
        ..Settings::default()
    };
    req(&mut e, W, Request::SetSettings { settings: s }, 0);
    assert!(!has(&e.core.handle(Input::Tick, 500_000), |f| matches!(
        f,
        Effect::Exit
    )));
    // Once the session is gone the countdown starts.
    e.core.handle(Input::PtyExited { target: st(&id) }, 500_000);
    assert!(has(&e.core.handle(Input::Tick, 510_000), |f| matches!(
        f,
        Effect::Exit
    )));
}

// ---- 12. memory -------------------------------------------------------

#[test]
fn memory_is_measured_periodically_while_a_window_is_connected() {
    let mut e = env();
    let a = new_session(&mut e, 0);
    let pid = e.pid.clone();
    // No window yet: nothing.
    assert!(e.core.handle(Input::Tick, 0).is_empty());
    e.core.handle(Input::WindowConnected(W), 1);
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        1,
    );
    let fx = e.core.handle(Input::Tick, 250);
    let targets = fx.iter().find_map(|f| {
        if let Effect::MeasureMemory { targets } = f {
            Some(targets.clone())
        } else {
            None
        }
    });
    // Claude sessions only: the footer counts awake sessions (spec §3.3).
    assert_eq!(targets, Some(vec![st(&a)]));
    assert!(!has(&e.core.handle(Input::Tick, 5249), |f| matches!(
        f,
        Effect::MeasureMemory { .. }
    )));
    assert!(has(&e.core.handle(Input::Tick, 5250), |f| matches!(
        f,
        Effect::MeasureMemory { .. }
    )));
}

#[test]
fn last_active_times_are_saved_lazily_at_most_every_30_s() {
    let mut e = env();
    let id = used_session(&mut e, 0);
    e.core.handle(Input::Tick, 30_000); // flushes the setup's touches
    let tool = format!(r#"{{"hook_event_name":"PreToolUse","session_id":"{id}"}}"#);
    let fx = hook(&mut e, &id, tool.clone(), 40_000);
    assert_eq!(saves(&fx), 0, "a touch alone does not save at once");
    hook(&mut e, &id, tool, 50_000);
    assert_eq!(saves(&e.core.handle(Input::Tick, 69_999)), 0);
    let fx = e.core.handle(Input::Tick, 70_000);
    let saved = fx.iter().find_map(|f| match f {
        Effect::Save { state } => Some(state.session(&id).unwrap().last_active_ms),
        _ => None,
    });
    assert_eq!(saved, Some(50_000));
    // Nothing pending: no more periodic saves.
    assert_eq!(saves(&e.core.handle(Input::Tick, 200_000)), 0);
    // Any other save carries the touch and resets the clock.
    hook(&mut e, &id, stop_json(&id), 210_000);
    req(
        &mut e,
        W,
        Request::Rename {
            deck_id: id.clone(),
            name: "x".into(),
        },
        211_000,
    );
    assert_eq!(saves(&e.core.handle(Input::Tick, 250_000)), 0);
}

#[test]
fn the_helper_saves_pending_touches_before_it_exits() {
    let mut e = env();
    let id = used_session(&mut e, 0);
    let fx = hook(&mut e, &id, stop_json(&id), 5); // only touches
    assert_eq!(saves(&fx), 0);
    e.core.handle(Input::PtyExited { target: st(&id) }, 6);
    let fx = e.core.handle(Input::Tick, 10_006);
    let exit = fx
        .iter()
        .position(|f| matches!(f, Effect::Exit))
        .expect("Exit");
    let save = fx
        .iter()
        .position(|f| matches!(f, Effect::Save { .. }))
        .expect("Save");
    assert!(save < exit);
}

#[test]
fn measured_memory_is_broadcast() {
    let mut e = env();
    let fx = e.core.handle(
        Input::MemoryMeasured {
            awake: 3,
            bytes: 1234,
        },
        0,
    );
    assert_eq!(
        events(&fx),
        vec![Event::Memory {
            awake: 3,
            bytes: 1234
        }]
    );
    assert!(matches!(fx[0], Effect::Broadcast { .. }));
}

// ---- 13. state --------------------------------------------------------

#[test]
fn hello_window_and_window_connected_and_get_state_reply_with_state() {
    let mut e = env();
    let fx = req(
        &mut e,
        W,
        Request::Hello {
            client: ClientKind::Window,
        },
        0,
    );
    assert!(matches!(fx.as_slice(), [Effect::Send { client: W, .. }]));
    assert!(matches!(events(&fx).as_slice(), [Event::State { .. }]));
    let fx = e.core.handle(Input::WindowConnected(W2), 0);
    assert!(matches!(fx.as_slice(), [Effect::Send { client: W2, .. }]));
    let fx = req(&mut e, W, Request::GetState, 0);
    assert!(matches!(events(&fx).as_slice(), [Event::State { .. }]));
    let fx = req(
        &mut e,
        HOOK,
        Request::Hello {
            client: ClientKind::Hook,
        },
        0,
    );
    assert!(fx.is_empty());
}

#[test]
fn state_lists_saved_sessions_with_live_status_or_asleep() {
    let mut e = env();
    let live = idle_session(&mut e, 0);
    let asleep = idle_session(&mut e, 0);
    e.core.handle(
        Input::PtyExited {
            target: st(&asleep),
        },
        1,
    );
    let state = get_state(&mut e);
    let find = |id: &str| {
        state
            .sessions
            .iter()
            .find(|s| s.deck_id == id)
            .unwrap()
            .state
    };
    assert_eq!(find(&live), SessionState::Idle);
    assert_eq!(find(&asleep), SessionState::Asleep);
    assert_eq!(state.projects.len(), 1);
    assert!(state.missing_folders.is_empty());
    assert!(state.shells.is_empty());
}

// ---- 14. projects -----------------------------------------------------

#[test]
fn add_project_saves_and_broadcasts_and_rejects_bad_paths() {
    let mut e = env();
    let other = tempfile::tempdir().unwrap();
    let fx = req(
        &mut e,
        W,
        Request::AddProject {
            path: other.path().to_str().unwrap().into(),
        },
        0,
    );
    assert_eq!(e.core.state().projects.len(), 2);
    assert_eq!(saves(&fx), 1);
    let Some(Event::State { state }) = events(&fx).into_iter().next() else {
        panic!()
    };
    assert_eq!(state.projects.len(), 2);
    assert!(matches!(fx.last(), Some(Effect::Broadcast { .. })));

    let missing = other.path().join("nope");
    let fx = req(
        &mut e,
        W,
        Request::AddProject {
            path: missing.to_str().unwrap().into(),
        },
        1,
    );
    assert_eq!(errors(&fx).len(), 1);
    assert_eq!(e.core.state().projects.len(), 2);
    let file = e.dir.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let fx = req(
        &mut e,
        W,
        Request::AddProject {
            path: file.to_str().unwrap().into(),
        },
        2,
    );
    assert_eq!(errors(&fx).len(), 1);
}

#[test]
fn remove_project_sleeps_sessions_kills_the_shell_and_forgets_everything() {
    let mut e = env();
    let busy = new_session(&mut e, 0);
    let pid = e.pid.clone();
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        0,
    );
    let fx = req(
        &mut e,
        W,
        Request::RemoveProject {
            project_id: pid.clone(),
            force: true,
        },
        5,
    );
    assert_eq!(writes(&fx, &st(&busy)), vec![[3u8].as_slice()]);
    assert_eq!(count_kills(&fx, &sh(&pid)), 1);
    assert!(e.core.state().projects.is_empty());
    assert!(e.core.state().sessions.is_empty());
    assert_eq!(saves(&fx), 1);
    assert!(matches!(events(&fx).last(), Some(Event::State { .. })));
    // Late exits are harmless.
    let fx = e.core.handle(Input::PtyExited { target: st(&busy) }, 6);
    assert!(events(&fx).is_empty());
    let fx = req(
        &mut e,
        W,
        Request::RemoveProject {
            project_id: pid,
            force: false,
        },
        7,
    );
    assert_eq!(errors(&fx), vec!["Unknown project".to_string()]);
}

#[test]
fn remove_project_with_a_busy_session_asks_unless_forced() {
    let mut e = env();
    let idle = idle_session(&mut e, 0);
    let busy = new_session(&mut e, 0);
    let pid = e.pid.clone();
    let remove = |force| Request::RemoveProject {
        project_id: pid.clone(),
        force,
    };
    let fx = req(&mut e, W, remove(false), 1);
    assert_eq!(
        confirms(&fx),
        vec![(
            ConfirmAction::RemoveProject {
                project_id: pid.clone()
            },
            "Claude may be working. Stop it anyway?".to_string()
        )]
    );
    // Nothing was stopped or forgotten.
    assert!(!touched(&fx, &idle) && !touched(&fx, &busy));
    assert!(e.core.state().project(&pid).is_some());
    assert_eq!(saves(&fx), 0);
    let fx = req(&mut e, W, remove(true), 2);
    assert_eq!(writes(&fx, &st(&busy)).len(), 1);
    assert_eq!(writes(&fx, &st(&idle)).len(), 1);
    assert!(e.core.state().project(&pid).is_none());
}

#[test]
fn remove_project_with_only_idle_sessions_needs_no_confirmation() {
    let mut e = env();
    let idle = idle_session(&mut e, 0);
    let pid = e.pid.clone();
    let remove = Request::RemoveProject {
        project_id: pid,
        force: false,
    };
    let fx = req(&mut e, W, remove, 1);
    assert!(confirms(&fx).is_empty());
    assert_eq!(writes(&fx, &st(&idle)).len(), 1);
}

#[test]
fn new_session_for_an_unknown_project_is_an_error() {
    let mut e = env();
    let fx = req(
        &mut e,
        W,
        Request::NewSession {
            project_id: "nope".into(),
        },
        0,
    );
    assert_eq!(errors(&fx).len(), 1);
    assert!(e.core.state().sessions.is_empty());
}

#[test]
fn open_past_session_adds_it_and_resumes_that_conversation() {
    let mut e = env();
    let pid = e.pid.clone();
    let fx = req(
        &mut e,
        W,
        Request::OpenPastSession {
            project_id: pid,
            conversation_id: "old-conv".into(),
        },
        7,
    );
    assert_eq!(e.core.state().sessions.len(), 1);
    let id = e.core.state().sessions[0].id.clone();
    assert_eq!(e.core.state().sessions[0].conversation_id, "old-conv");
    assert_eq!(saves(&fx), 1);
    let sp = spawns(&fx);
    assert_eq!(sp[0].0, &st(&id));
    assert_eq!(
        sp[0].1.args,
        ["--resume", "old-conv", "--settings", "C:/hooks.json"]
    );
    assert!(e.core.state().sessions[0].has_conversation);
}

#[test]
fn opening_a_past_conversation_that_is_already_open_does_not_spawn_again() {
    let mut e = env();
    let id = used_session(&mut e, 0);
    let pid = e.pid.clone();
    let open = Request::OpenPastSession {
        project_id: pid,
        conversation_id: id.clone(),
    };
    let fx = req(&mut e, W, open, 1);
    assert!(spawns(&fx).is_empty());
    assert_eq!(e.core.state().sessions.len(), 1);
}

#[test]
fn esc_on_a_permission_prompt_then_idle_prompt_lets_the_session_auto_sleep() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    attach(&mut e, W, st(&id), 10, 10);
    hook(&mut e, &id, prompt_json(&id, "write x"), 1);
    hook(&mut e, &id, ask_json(&id), 2);
    // Esc sends no hook (spike 2); the key press alone ends Needs you.
    let esc = Input::TerminalInput {
        index: 0,
        bytes: b"\x1b".to_vec(),
    };
    e.core.handle(esc, 3);
    assert_eq!(session_view(&mut e, &id).state, SessionState::Working);
    let idle = format!(
        r#"{{"hook_event_name":"Notification","session_id":"{id}","notification_type":"idle_prompt","message":"Claude is waiting for your input"}}"#
    );
    let at = 63_000;
    hook(&mut e, &id, idle, at);
    assert_eq!(session_view(&mut e, &id).state, SessionState::Idle);
    let fx = e.core.handle(Input::Tick, at + 30 * 60_000);
    assert_eq!(writes(&fx, &st(&id)), vec![[3u8].as_slice()]);
}

#[test]
fn rename_saves_and_broadcasts_the_session() {
    let mut e = env();
    let id = new_session(&mut e, 0);
    let fx = req(
        &mut e,
        W,
        Request::Rename {
            deck_id: id.clone(),
            name: "  My work ".into(),
        },
        1,
    );
    assert_eq!(saves(&fx), 1);
    let session = events(&fx)
        .into_iter()
        .find_map(|ev| {
            if let Event::SessionChanged { session } = ev {
                Some(session)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(session.name.as_deref(), Some("My work"));
    // A later prompt never overwrites it.
    hook(&mut e, &id, prompt_json(&id, "something else"), 2);
    assert_eq!(
        e.core.state().session(&id).unwrap().name.as_deref(),
        Some("My work")
    );
}

// ---- 15. past sessions and git -----------------------------------------

#[test]
fn list_past_excludes_open_conversations_and_replies_to_the_asker() {
    let mut e = env();
    let id = idle_session(&mut e, 0);
    let pid = e.pid.clone();
    let fx = req(
        &mut e,
        W2,
        Request::ListPastSessions {
            project_id: pid.clone(),
        },
        1,
    );
    match fx.as_slice() {
        [Effect::ListPast {
            client,
            project_id,
            path,
            exclude,
        }] => {
            assert_eq!((*client, project_id), (W2, &pid));
            assert_eq!(path, e.dir.path());
            assert!(exclude.contains(&id));
        }
        other => panic!("{other:?}"),
    }
    let sessions = vec![PastSession {
        id: "x".into(),
        last_prompt: Some("p".into()),
        modified_ms: 5,
    }];
    let fx = e.core.handle(
        Input::PastDone {
            client: W2,
            project_id: pid.clone(),
            sessions: sessions.clone(),
        },
        2,
    );
    assert!(matches!(fx.as_slice(), [Effect::Send { client: W2, .. }]));
    assert_eq!(
        events(&fx),
        vec![Event::PastSessions {
            project_id: pid,
            sessions
        }]
    );
}

#[test]
fn git_status_runs_in_the_project_folder_and_replies_to_the_asker() {
    let mut e = env();
    let pid = e.pid.clone();
    let fx = req(
        &mut e,
        W2,
        Request::GitStatus {
            project_id: pid.clone(),
        },
        0,
    );
    match fx.as_slice() {
        [Effect::RunGit {
            client: W2,
            project_id,
            path,
        }] => {
            assert_eq!(project_id, &pid);
            assert_eq!(path, e.dir.path());
        }
        other => panic!("{other:?}"),
    }
    let fx = e.core.handle(
        Input::GitDone {
            client: W2,
            project_id: pid.clone(),
            result: GitResult::NotGit,
        },
        1,
    );
    assert!(matches!(fx.as_slice(), [Effect::Send { client: W2, .. }]));
    assert_eq!(
        events(&fx),
        vec![Event::GitStatus {
            project_id: pid,
            result: GitResult::NotGit
        }]
    );
    let fx = req(
        &mut e,
        W,
        Request::GitStatus {
            project_id: "nope".into(),
        },
        2,
    );
    assert_eq!(errors(&fx).len(), 1);
}

// ---- 16. settings and panel --------------------------------------------

#[test]
fn settings_and_panel_changes_save_and_broadcast_state() {
    let mut e = env();
    let s = Settings {
        auto_sleep_minutes: Some(60),
        ..Settings::default()
    };
    let fx = req(&mut e, W, Request::SetSettings { settings: s }, 0);
    assert_eq!(e.core.state().settings.auto_sleep_minutes, Some(60));
    assert_eq!(saves(&fx), 1);
    assert!(matches!(events(&fx).as_slice(), [Event::State { .. }]));
    assert!(matches!(fx.last(), Some(Effect::Broadcast { .. })));

    let pid = e.pid.clone();
    let fx = req(
        &mut e,
        W,
        Request::SetShellPanel {
            project_id: pid.clone(),
            panel: ShellPanel {
                open: true,
                width: 600,
            },
        },
        1,
    );
    assert_eq!(
        e.core.state().project(&pid).unwrap().shell_panel,
        ShellPanel {
            open: true,
            width: 600
        }
    );
    assert_eq!(saves(&fx), 1);
    let Some(Event::State { state }) = events(&fx).into_iter().next() else {
        panic!()
    };
    assert!(state.projects[0].shell_panel.open);
}

#[test]
fn close_shell_kills_that_shell_once() {
    let mut e = env();
    let pid = e.pid.clone();
    req(
        &mut e,
        W,
        Request::Attach {
            target: sh(&pid),
            rows: 10,
            cols: 10,
        },
        0,
    );
    let fx = req(
        &mut e,
        W,
        Request::CloseShell {
            project_id: pid.clone(),
        },
        1,
    );
    assert_eq!(count_kills(&fx, &sh(&pid)), 1);
    let fx = req(
        &mut e,
        W,
        Request::CloseShell {
            project_id: pid.clone(),
        },
        2,
    );
    assert_eq!(count_kills(&fx, &sh(&pid)), 0);
    // Closing a shell that does not run does nothing.
    let fx = req(
        &mut e,
        W,
        Request::CloseShell {
            project_id: "nope".into(),
        },
        3,
    );
    assert!(fx.is_empty());
}

#[test]
fn set_project_layout_saves_broadcasts_and_clears() {
    let mut e = env();
    let pid = e.pid.clone();
    let lay = serde_json::json!({"type":"pane","id":"p1","tabs":[],"active":0});
    let fx = req(
        &mut e,
        W,
        Request::SetProjectLayout {
            project_id: pid.clone(),
            layout: Some(lay.clone()),
        },
        1,
    );
    assert_eq!(saves(&fx), 1);
    assert!(errors(&fx).is_empty());
    let Some(Event::State { state }) = events(&fx).into_iter().next() else {
        panic!()
    };
    assert_eq!(state.projects[0].layout, Some(lay));
    assert!(fx.iter().any(|f| matches!(f, Effect::Broadcast { .. })));
    let fx = req(
        &mut e,
        W,
        Request::SetProjectLayout {
            project_id: pid.clone(),
            layout: None,
        },
        2,
    );
    assert_eq!(saves(&fx), 1);
    assert_eq!(e.core.state().project(&pid).unwrap().layout, None);
}

#[test]
fn set_project_layout_errors_go_to_the_requester_only() {
    let mut e = env();
    let pid = e.pid.clone();
    let fx = req(
        &mut e,
        W,
        Request::SetProjectLayout {
            project_id: "nope".into(),
            layout: None,
        },
        1,
    );
    assert_eq!(errors(&fx), vec!["unknown id".to_string()]);
    assert_eq!(saves(&fx), 0);
    assert!(matches!(
        fx.as_slice(),
        [Effect::Send { client, .. }] if *client == W
    ));
    let fx = req(
        &mut e,
        W,
        Request::SetProjectLayout {
            project_id: pid,
            layout: Some(serde_json::json!([])),
        },
        2,
    );
    assert_eq!(
        errors(&fx),
        vec!["layout must be a JSON object".to_string()]
    );
    assert_eq!(saves(&fx), 0);
}

#[test]
fn add_group_saves_and_appears_in_state() {
    let mut e = env();
    let fx = req(
        &mut e,
        W,
        Request::AddGroup {
            name: " Billing ".into(),
        },
        1,
    );
    assert_eq!(saves(&fx), 1);
    assert!(fx.iter().any(|f| matches!(f, Effect::Broadcast { .. })));
    let Some(Event::State { state }) = events(&fx).into_iter().next() else {
        panic!()
    };
    assert_eq!(state.groups.len(), 1);
    assert_eq!(state.groups[0].name, "Billing");
    assert_eq!(state.groups[0].layout, serde_json::json!({}));
    req(&mut e, W, Request::AddGroup { name: "".into() }, 2);
    assert_eq!(get_state(&mut e).groups[1].name, "Group 2");
}

#[test]
fn move_project_saves_and_broadcasts_the_new_order() {
    let mut e = env();
    let other = tempfile::tempdir().unwrap();
    req(
        &mut e,
        W,
        Request::AddProject {
            path: other.path().to_str().unwrap().into(),
        },
        0,
    );
    let pid = e.pid.clone();
    let fx = req(
        &mut e,
        W,
        Request::MoveProject {
            project_id: pid.clone(),
            index: 1,
        },
        1,
    );
    assert_eq!((saves(&fx), errors(&fx).len()), (1, 0));
    let Some(Event::State { state }) = events(&fx).into_iter().next() else {
        panic!()
    };
    assert_eq!(state.projects[1].id, pid);
    let fx = req(
        &mut e,
        W,
        Request::MoveProject {
            project_id: "nope".into(),
            index: 0,
        },
        2,
    );
    assert_eq!(errors(&fx), vec!["unknown id".to_string()]);
    assert_eq!(saves(&fx), 0);
}

#[test]
fn rename_set_layout_and_remove_group_each_save_and_broadcast() {
    let mut e = env();
    req(&mut e, W, Request::AddGroup { name: "A".into() }, 1);
    let gid = e.core.state().groups[0].id.clone();
    let fx = req(
        &mut e,
        W,
        Request::RenameGroup {
            group_id: gid.clone(),
            name: "B".into(),
        },
        2,
    );
    assert_eq!((saves(&fx), errors(&fx).len()), (1, 0));
    assert_eq!(get_state(&mut e).groups[0].name, "B");
    let lay = serde_json::json!({"type":"pane","id":"p1","tabs":[],"active":0});
    let fx = req(
        &mut e,
        W,
        Request::SetGroupLayout {
            group_id: gid.clone(),
            layout: lay.clone(),
        },
        3,
    );
    assert_eq!((saves(&fx), errors(&fx).len()), (1, 0));
    let Some(Event::State { state }) = events(&fx).into_iter().next() else {
        panic!()
    };
    assert_eq!(state.groups[0].layout, lay);
    let fx = req(&mut e, W, Request::RemoveGroup { group_id: gid }, 4);
    assert_eq!((saves(&fx), errors(&fx).len()), (1, 0));
    assert!(get_state(&mut e).groups.is_empty());
}

#[test]
fn group_errors_go_to_the_requester_without_saving() {
    let mut e = env();
    req(&mut e, W, Request::AddGroup { name: "A".into() }, 1);
    let gid = e.core.state().groups[0].id.clone();
    let fx = req(
        &mut e,
        W,
        Request::SetGroupLayout {
            group_id: gid,
            layout: serde_json::json!([1]),
        },
        2,
    );
    assert_eq!(
        errors(&fx),
        vec!["layout must be a JSON object".to_string()]
    );
    assert_eq!(saves(&fx), 0);
    assert!(!fx.iter().any(|f| matches!(f, Effect::Broadcast { .. })));
    let fx = req(
        &mut e,
        W,
        Request::RemoveGroup {
            group_id: "nope".into(),
        },
        3,
    );
    assert_eq!(errors(&fx), vec!["unknown id".to_string()]);
    assert_eq!(saves(&fx), 0);
    let fx = req(
        &mut e,
        W,
        Request::RenameGroup {
            group_id: "nope".into(),
            name: "x".into(),
        },
        4,
    );
    assert_eq!(errors(&fx), vec!["unknown id".to_string()]);
    let fx = req(
        &mut e,
        W,
        Request::SetGroupLayout {
            group_id: "nope".into(),
            layout: serde_json::json!({}),
        },
        5,
    );
    assert_eq!(errors(&fx), vec!["unknown id".to_string()]);
    assert_eq!(e.core.state().groups.len(), 1);
}
