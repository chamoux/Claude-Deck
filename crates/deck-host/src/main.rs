//! `deck-host serve` (default) runs the helper; `deck-host hook` is the Claude hook command.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use deck_host::host::core::{Core, CoreConfig};
use deck_host::host::runtime::{self, RuntimeConfig};
use deck_host::{hook_cmd, locate, pipe};

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("hook") => {
            hook();
            ExitCode::SUCCESS
        }
        _ => serve(),
    }
}

/// `CLAUDE_DECK_PIPE` is honoured only for real local pipe names; anything else is ignored.
fn pipe_override(v: Option<String>) -> Option<String> {
    v.filter(|n| n.starts_with(r"\\.\pipe\"))
}

/// Another helper may be shutting down right now; give it a moment before giving up.
fn acquire_single_instance(sid: &str) -> std::io::Result<Option<pipe::SingleInstance>> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match pipe::SingleInstance::acquire(sid)? {
            Some(g) => return Ok(Some(g)),
            None if Instant::now() >= deadline => return Ok(None),
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Always exits 0: a failing hook must never disturb Claude.
fn hook() {
    let stdin = std::io::read_to_string(std::io::stdin()).unwrap_or_default();
    let deck_id = std::env::var("CLAUDE_DECK_SESSION").ok();
    // The helper passes its pipe name to the sessions it starts, so tests (and
    // any non-default helper) get their own hooks. Default: this user's pipe.
    let name = match pipe_override(std::env::var("CLAUDE_DECK_PIPE").ok()) {
        Some(n) => n,
        None => {
            let Ok(sid) = pipe::current_user_sid() else {
                return;
            };
            pipe::pipe_name(&sid)
        }
    };
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    rt.block_on(hook_cmd::run_hook(stdin, deck_id, &name));
}

fn fail(msg: &str) -> ExitCode {
    runtime::log(msg);
    eprintln!("{msg}");
    ExitCode::from(1)
}

fn serve() -> ExitCode {
    // A parent may have set "ignore Ctrl+C"; children inherit it and \x03 would do nothing.
    // SAFETY: plain Win32 call, no pointers involved.
    unsafe {
        let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(None, false);
    }
    let sid = match pipe::current_user_sid() {
        Ok(s) => s,
        Err(e) => return fail(&format!("cannot read user SID: {e}")),
    };
    let _guard = match acquire_single_instance(&sid) {
        Ok(Some(g)) => g,
        Ok(None) => {
            println!("already running");
            return ExitCode::SUCCESS;
        }
        Err(e) => return fail(&format!("single-instance check failed: {e}")),
    };
    let app_dir = match locate::app_dir() {
        Ok(d) => d,
        Err(e) => return fail(&format!("no app dir: {e}")),
    };
    runtime::init_logging(&locate::logs_dir(&app_dir));
    runtime::install_panic_hook();
    let clock = runtime::system_clock();
    let state_path = locate::state_path(&app_dir);
    // On error, stop: defaults must never be saved over a file we could not read.
    let loaded = match deck_core::store::load(&state_path, clock()) {
        Ok(l) => l,
        Err(e) => return fail(&format!("cannot load state: {e}")),
    };
    if let Some(b) = &loaded.corrupt_backup {
        runtime::log(&format!("state file was corrupt, moved to {}", b.display()));
    }
    let hooks_file = locate::hooks_path(&app_dir);
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return fail(&format!("no current exe: {e}")),
    };
    if let Err(e) = hook_cmd::write_hooks_file(&hooks_file, &exe) {
        return fail(&format!("cannot write hooks file: {e}"));
    }
    let settings = &loaded.state.settings;
    let claude_exe = locate::find_claude(settings.claude_path.as_deref());
    runtime::log(&match &claude_exe {
        Some(p) => format!("claude.exe: {}", p.display()),
        None => "claude.exe not found, falling back to PATH lookup".to_string(),
    });
    let config = CoreConfig {
        claude_exe: claude_exe.unwrap_or_else(|| PathBuf::from("claude.exe")),
        hooks_file,
        shell: locate::shell_command(settings.shell),
        ctrl_c_gap_ms: 300,
        sleep_grace_ms: 5000,
        idle_exit_ms: 10_000,
        memory_every_ms: 5000,
        pipe_name: Some(pipe::pipe_name(&sid)),
    };
    let security = match pipe::PipeSecurity::only(&sid) {
        Ok(s) => Arc::new(s),
        Err(e) => return fail(&format!("cannot build pipe security: {e}")),
    };
    let projects = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".claude")
        .join("projects");
    let cfg = RuntimeConfig {
        pipe_name: pipe::pipe_name(&sid),
        security,
        state_path,
        claude_projects_dir: projects,
        tick_ms: 250,
        clock,
        max_window_queue_bytes: runtime::DEFAULT_MAX_WINDOW_QUEUE,
    };
    // The project shell follows the shell setting, also after it changes.
    let core = Core::new(loaded.state, config).with_shell_lookup(locate::shell_command);
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => return fail(&format!("cannot start runtime: {e}")),
    };
    runtime::log("host started");
    let result = rt.block_on(runtime::run(core, cfg));
    // Blocking tasks (git, saves) must not hold the process open forever.
    rt.shutdown_timeout(Duration::from_secs(2));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&format!("host failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::pipe_override;

    #[test]
    fn pipe_override_accepts_only_local_pipe_names() {
        let ok = r"\\.\pipe\claude-deck-x".to_string();
        assert_eq!(pipe_override(Some(ok.clone())), Some(ok));
        assert_eq!(pipe_override(None), None);
        assert_eq!(pipe_override(Some(String::new())), None);
        assert_eq!(pipe_override(Some(r"\\evil-host\pipe\x".into())), None);
        assert_eq!(pipe_override(Some("claude-deck".into())), None);
    }
}
