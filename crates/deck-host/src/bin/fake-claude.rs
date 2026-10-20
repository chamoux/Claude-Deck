//! Stand-in for `claude.exe` used by the integration tests. It runs the hooks
//! from its `--settings` file through `bash -c`, the way Claude Code does.

use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use windows::core::BOOL;
use windows::Win32::System::Console::SetConsoleCtrlHandler;

static START: OnceLock<Instant> = OnceLock::new();
/// Millis since START of the last Ctrl+C event, plus one (0 = none yet).
static LAST_CTRL_C: AtomicU64 = AtomicU64::new(0);
static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn ctrl_handler(_ctrl_type: u32) -> BOOL {
    let now = START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1;
    let prev = LAST_CTRL_C.swap(now, Ordering::SeqCst);
    if prev != 0 && now - prev <= 2000 {
        EXIT_REQUESTED.store(true, Ordering::SeqCst);
    }
    BOOL(1)
}

struct Fake {
    session_id: String,
    settings: Value,
}

impl Fake {
    fn run_hooks(&self, event: &str, extra: Value) {
        let mut payload = json!({
            "session_id": self.session_id,
            "transcript_path": format!("C:/tmp/fake-claude/{}.jsonl", self.session_id),
            "cwd": std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            "hook_event_name": event,
        });
        if let (Some(p), Some(e)) = (payload.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                p.insert(k.clone(), v.clone());
            }
        }
        let tool = payload
            .get("tool_name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let body = payload.to_string();
        let Some(groups) = self
            .settings
            .pointer(&format!("/hooks/{event}"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for group in groups {
            if matches!(event, "PreToolUse" | "PostToolUse") {
                let m = group.get("matcher").and_then(Value::as_str).unwrap_or("");
                if !(m.is_empty() || m == "*" || Some(m) == tool.as_deref()) {
                    continue;
                }
            }
            let Some(hooks) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for h in hooks {
                if h.get("type").and_then(Value::as_str) != Some("command") {
                    continue;
                }
                if let Some(cmd) = h.get("command").and_then(Value::as_str) {
                    run_command(cmd, &body);
                }
            }
        }
    }

    fn end_and_exit(&self, reason: &str) -> ! {
        self.run_hooks("SessionEnd", json!({ "reason": reason }));
        std::process::exit(0);
    }
}

fn bash_path() -> String {
    let p = r"C:\Program Files\Git\bin\bash.exe";
    if std::path::Path::new(p).exists() {
        p.to_string()
    } else {
        "bash".to_string()
    }
}

fn run_command(cmd: &str, stdin_json: &str) {
    let Ok(mut child) = Command::new(bash_path())
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Own console: a hook's bash must not share the ConPTY console and catch its Ctrl+C.
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
    else {
        return;
    };
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(stdin_json.as_bytes());
    }
    let _ = child.wait();
}

fn out(s: &str) {
    let mut o = std::io::stdout();
    let _ = o.write_all(s.as_bytes());
    let _ = o.flush();
}

/// Line reader over chunks fed by the stdin thread.
struct Input {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    eof: bool,
}

/// Removes `ESC [ <digits> ; <digits> R` cursor reports. ConPTY's own startup
/// query is answered too, so a second report can land in front of the first
/// command line.
fn strip_cursor_reports(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b';') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'R' && j > i + 2 {
                i = j + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

impl Input {
    fn take_line(&mut self) -> Option<String> {
        loop {
            let pos = self.buf.iter().position(|&b| b == b'\n' || b == b'\r')?;
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = strip_cursor_reports(&line[..line.len() - 1]);
            let s = String::from_utf8_lossy(&line).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    /// Next non-empty line; None on EOF. A Ctrl+C double-tap exits via `fake`.
    fn next_line(&mut self, fake: &Fake) -> Option<String> {
        loop {
            if EXIT_REQUESTED.load(Ordering::SeqCst) {
                fake.end_and_exit("prompt_input_exit");
            }
            if let Some(l) = self.take_line() {
                return Some(l);
            }
            if self.eof {
                return None;
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(c) => self.buf.extend(c),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => self.eof = true,
            }
        }
    }
}

fn main() {
    START.get_or_init(Instant::now);
    unsafe {
        let _ = SetConsoleCtrlHandler(None, false);
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), true);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut session, mut resumed, mut settings_path) = (None, false, None);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--session-id" if i + 1 < args.len() => {
                session = Some(args[i + 1].clone());
                i += 1;
            }
            "--resume" if i + 1 < args.len() => {
                session = Some(args[i + 1].clone());
                resumed = true;
                i += 1;
            }
            "--settings" if i + 1 < args.len() => {
                settings_path = Some(args[i + 1].clone());
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let settings = settings_path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null);
    let mut fake = Fake {
        session_id: session.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        settings,
    };

    out("fake-claude ready\r\n\x1b[6n");

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut si = std::io::stdin();
        let mut b = [0u8; 256];
        while let Ok(n) = si.read(&mut b) {
            if n == 0 || tx.send(b[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut input = Input {
        rx,
        buf: Vec::new(),
        eof: false,
    };

    // Wait up to 2 s for the cursor report (ends in 'R'); keep everything on timeout.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut pre: Vec<u8> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match input.rx.recv_timeout(left) {
            Ok(c) => {
                pre.extend(c);
                if let Some(p) = pre.iter().position(|&b| b == b'R') {
                    pre.drain(..=p);
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => {
                input.eof = true;
                break;
            }
        }
    }
    input.buf = pre;

    fake.run_hooks(
        "SessionStart",
        json!({ "source": if resumed { "resume" } else { "startup" } }),
    );
    out("> ");

    let empty = || json!({ "background_tasks": [], "session_crons": [] });
    while let Some(line) = input.next_line(&fake) {
        let cmd = line.split_whitespace().next().unwrap_or("");
        match cmd {
            "work" => {
                let n: u32 = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                fake.run_hooks("UserPromptSubmit", json!({ "prompt": line }));
                for _ in 0..n {
                    out(".");
                    std::thread::sleep(Duration::from_millis(100));
                }
                fake.run_hooks("Stop", empty());
                out("done\r\n> ");
            }
            "background" => {
                fake.run_hooks("UserPromptSubmit", json!({ "prompt": line }));
                fake.run_hooks(
                    "Stop",
                    json!({ "background_tasks": [{"id":"t1","type":"shell","status":"running"}], "session_crons": [] }),
                );
                out("> ");
            }
            "ask" => {
                fake.run_hooks("UserPromptSubmit", json!({ "prompt": line }));
                fake.run_hooks(
                    "PreToolUse",
                    json!({ "tool_name": "Write", "tool_input": {} }),
                );
                fake.run_hooks(
                    "Notification",
                    json!({ "notification_type": "permission_prompt", "message": "Claude needs your permission" }),
                );
                let ans = input.next_line(&fake);
                if ans.as_deref() == Some("y") {
                    fake.run_hooks(
                        "PostToolUse",
                        json!({ "tool_name": "Write", "tool_input": {}, "tool_response": {} }),
                    );
                }
                fake.run_hooks("Stop", empty());
                out("> ");
            }
            "clear" => {
                fake.run_hooks("SessionEnd", json!({ "reason": "clear" }));
                fake.session_id = uuid::Uuid::new_v4().to_string();
                fake.run_hooks("SessionStart", json!({ "source": "clear" }));
                out("> ");
            }
            "exit" => fake.end_and_exit("other"),
            _ => out(&format!("{line}\r\n> ")),
        }
    }
    fake.end_and_exit("other");
}
