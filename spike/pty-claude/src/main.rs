//! Spike 3: drive the real `claude` CLI through our ConPTY code.
//! Usage: pty-claude <a|b|c>   (a = transcript/resume, b = late permission Notification, c = /clear then resume)
use deck_host::dsr::{cursor_report, DsrFilter};
use deck_host::pty::{PtyEvent, PtyProcess, SpawnSpec};
use deck_host::screen::Screen;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LOG: &str = "C:/Users/Chamoux/Desktop/claude-deck/spike/hooks.log";
const SETTINGS: &str = "C:/Users/Chamoux/Desktop/claude-deck/spike/hooks.json";

struct State {
    screen: Screen,
    filter: DsrFilter,
    exited: Option<(Instant, Option<u32>)>,
}

struct Sess {
    st: Arc<Mutex<State>>,
    pty: Arc<PtyProcess>,
    t0: Instant,
}

impl Sess {
    fn screen(&self) -> String {
        self.st.lock().unwrap().screen.contents()
    }
    fn alive(&self) -> bool {
        self.st.lock().unwrap().exited.is_none()
    }
    fn exit_code(&self) -> Option<Option<u32>> {
        self.st.lock().unwrap().exited.map(|(_, c)| c)
    }
    fn w(&self, b: &[u8]) {
        let _ = self.pty.write(b);
    }
    fn wait_exit(&self, secs: u64) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if !self.alive() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        !self.alive()
    }
    /// Ctrl+C, 300 ms, Ctrl+C; wait for exit.
    fn quit(&self) -> (bool, f64) {
        let t = Instant::now();
        self.w(b"\x03");
        std::thread::sleep(Duration::from_millis(300));
        self.w(b"\x03");
        let ok = self.wait_exit(10);
        (ok, t.elapsed().as_secs_f64())
    }
}

fn lines() -> Vec<String> {
    std::fs::read_to_string(LOG).unwrap_or_default().lines().map(|l| l.replace("\": ", "\":")).collect()
}

fn wait_hook(name: &str, from: usize, secs: u64, extra: &str) -> Option<String> {
    let end = Instant::now() + Duration::from_secs(secs);
    let pat = format!("\"hook_event_name\":\"{name}\"");
    loop {
        for line in lines().iter().skip(from) {
            if line.contains(&pat) && line.contains(extra) {
                return Some(line.clone());
            }
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn show_new(from: usize, label: &str) -> usize {
    let l = lines();
    for x in l.iter().skip(from) {
        println!("  [{label}] HOOK {x}");
    }
    l.len()
}

fn session_id_of(line: &str) -> String {
    let k = "\"session_id\":\"";
    let i = line.find(k).map(|i| i + k.len()).unwrap_or(0);
    line[i..].chars().take_while(|c| *c != '"').collect()
}

fn new_uuid() -> String {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        (n >> 64) as u32,
        (n >> 48) as u16,
        (n >> 32) as u16 & 0xfff,
        (n >> 16) as u16 & 0xfff,
        n as u64 & 0xffff_ffff_ffff
    )
}

fn spawn(args: Vec<String>, t0: Instant) -> Sess {
    let cwd = PathBuf::from(r"C:\tmp\deck-spike3");
    std::fs::create_dir_all(&cwd).unwrap();
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let p = home.join(".local/bin/claude.exe");
    let program = if p.exists() { p } else { PathBuf::from("claude.exe") };
    let mut a = args;
    a.push("--settings".into());
    a.push(SETTINGS.into());
    let spec = SpawnSpec {
        program,
        args: a,
        cwd,
        env: vec![
            ("CLAUDE_DECK_SESSION".into(), "spike-3b".into()),
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
        ],
        env_remove: vec!["CLAUDE_CODE_CHILD_SESSION".into(), "CLAUDECODE".into()],
        rows: 40,
        cols: 120,
        kill_tree: true,
    };
    let st = Arc::new(Mutex::new(State { screen: Screen::new(40, 120, 5000), filter: DsrFilter::new(), exited: None }));
    let cell: Arc<OnceLock<Arc<PtyProcess>>> = Arc::new(OnceLock::new());
    let (st2, cell2) = (st.clone(), cell.clone());
    let pty = PtyProcess::spawn(&spec, move |ev| match ev {
        PtyEvent::Output(b) => {
            let mut s = st2.lock().unwrap();
            let (clean, q) = s.filter.feed(&b);
            for _ in 0..q {
                let (r, c) = s.screen.cursor();
                if let Some(p) = cell2.get() {
                    let _ = p.write(&cursor_report(r, c));
                }
            }
            s.screen.process(&clean);
        }
        PtyEvent::Exited(c) => {
            st2.lock().unwrap().exited = Some((Instant::now(), c));
        }
    })
    .expect("spawn");
    let _ = cell.set(pty.clone());
    Sess { st, pty, t0 }
}

/// Wait for a SessionStart hook (new log lines after `ll`), accepting the trust prompt if shown.
fn wait_start(s: &Sess, ll: usize, secs: u64) -> Option<String> {
    let end = Instant::now() + Duration::from_secs(secs);
    let mut trusted = 0;
    while Instant::now() < end && s.alive() {
        std::thread::sleep(Duration::from_millis(500));
        if let Some(l) = wait_hook("SessionStart", ll, 0, "") {
            return Some(l);
        }
        let sc = s.screen();
        let low = sc.to_lowercase();
        if low.contains("yes, i trust") && trusted < 2 {
            println!("[{:.2}s] trust prompt, answering Down+Enter", s.t0.elapsed().as_secs_f64());
            std::thread::sleep(Duration::from_millis(1000));
            for _ in 0..3 {
                s.w(b"\x1b[B");
                std::thread::sleep(Duration::from_millis(600));
                if s.screen().contains("❯ Yes") {
                    break;
                }
            }
            s.w(b"\r");
            trusted += 1;
            std::thread::sleep(Duration::from_millis(1500));
        }
    }
    None
}

/// Spawn with args (resume-style), watch up to `secs` for exit or SessionStart; report.
fn probe(label: &str, args: Vec<String>) -> usize {
    println!("=== {label}: claude {}", args.join(" "));
    let t = Instant::now();
    let ll = lines().len();
    let s = spawn(args, t);
    let end = Instant::now() + Duration::from_secs(25);
    let mut start_at = None;
    let mut exit_at = None;
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
        if start_at.is_none() && wait_hook("SessionStart", ll, 0, "").is_some() {
            start_at = Some(t.elapsed().as_secs_f64());
        }
        if let Some((at, c)) = s.st.lock().unwrap().exited {
            exit_at = Some((at.duration_since(t).as_secs_f64(), c));
            break;
        }
        if start_at.is_some() {
            std::thread::sleep(Duration::from_secs(3));
            break;
        }
    }
    println!("{label}: SessionStart at {:?}s, exit {:?}", start_at, exit_at);
    println!("--- screen ({label}) ---\n{}\n---", s.screen());
    let l = show_new(ll, label);
    if s.alive() {
        let q = s.quit();
        println!("{label}: quit via Ctrl+C x2 -> exited={} in {:.2}s code {:?}", q.0, q.1, s.exit_code());
        std::thread::sleep(Duration::from_secs(2));
        if !q.0 {
            let _ = s.pty.kill();
        }
    }
    show_new(l, &format!("{label}-end"))
}

fn jsonl_report(id: &str) {
    let root = PathBuf::from(std::env::var("USERPROFILE").unwrap()).join(".claude/projects");
    for e in std::fs::read_dir(&root).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if n.contains("deck-spike3") {
            let f = e.path().join(format!("{id}.jsonl"));
            println!("project dir {:?}; {id}.jsonl exists={} size={:?}", e.file_name(), f.exists(), std::fs::metadata(&f).map(|m| m.len()).ok());
            for x in std::fs::read_dir(e.path()).unwrap().flatten() {
                println!("   entry {:?} {} bytes", x.file_name(), x.metadata().map(|m| m.len()).unwrap_or(0));
            }
        }
    }
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "a".into());
    let t0 = Instant::now();
    let ts = move || format!("[{:>7.2}s]", t0.elapsed().as_secs_f64());
    let x = new_uuid();
    println!("scenario {which}, session-id X = {x}");
    let ll0 = lines().len();
    match which.as_str() {
        "a" => {
            let s = spawn(vec!["--session-id".into(), x.clone(), "--permission-mode".into(), "default".into()], t0);
            let st = wait_start(&s, ll0, 40);
            println!("{} A1 SessionStart: {:?}", ts(), st);
            if st.is_none() {
                println!("BLOCKED screen:\n{}", s.screen());
                let _ = s.pty.kill();
                return;
            }
            std::thread::sleep(Duration::from_secs(3));
            show_new(ll0, "start");
            let q = s.quit();
            println!("{} A3 quit: exited={} in {:.2}s code {:?}", ts(), q.0, q.1, s.exit_code());
            std::thread::sleep(Duration::from_secs(2));
            show_new(ll0, "after-quit-all");
            jsonl_report(&x);
            let l = probe("A5 resume", vec!["--resume".into(), x.clone()]);
            jsonl_report(&x);
            let _ = l;
            probe("A6 session-id-again", vec!["--session-id".into(), x.clone()]);
            jsonl_report(&x);
        }
        "b" => {
            let s = spawn(vec!["--session-id".into(), x.clone(), "--permission-mode".into(), "default".into()], t0);
            let st = wait_start(&s, ll0, 40);
            println!("{} B SessionStart: {}", ts(), st.is_some());
            if st.is_none() {
                println!("BLOCKED screen:\n{}", s.screen());
                let _ = s.pty.kill();
                return;
            }
            std::thread::sleep(Duration::from_secs(3));
            let mut ll = lines().len();
            s.w(b"create a file y.txt containing 2");
            std::thread::sleep(Duration::from_millis(500));
            let wall = SystemTime::now();
            s.w(b"\r");
            println!("{} B1 prompt sent (wall {:?})", ts(), wall.duration_since(UNIX_EPOCH).unwrap().as_millis());
            let end = Instant::now() + Duration::from_secs(60);
            let mut seen = None;
            let mut shown = ll;
            while Instant::now() < end {
                std::thread::sleep(Duration::from_millis(100));
                let sc = s.screen();
                if sc.contains("Do you want to create") || sc.contains("Do you want to") {
                    seen = Some(s.t0.elapsed());
                    break;
                }
                let n = lines().len();
                if n > shown {
                    println!("{} (pre-dialog hooks)", ts());
                    shown = show_new(shown, "pre");
                }
            }
            println!("{} B2 dialog seen={:?}\n--- screen ---\n{}\n---", ts(), seen, s.screen());
            if seen.is_some() {
                s.w(b"1\r");
                println!("{} B2 wrote 1+CR (wall {:?})", ts(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis());
            }
            // poll hooks for 15 s with timestamps
            let end = Instant::now() + Duration::from_secs(15);
            while Instant::now() < end {
                std::thread::sleep(Duration::from_millis(100));
                let n = lines().len();
                if n > shown {
                    print!("{} ", ts());
                    shown = show_new(shown, "post");
                }
            }
            ll = shown;
            let _ = ll;
            println!("--- screen after 15s ---\n{}\n---", s.screen());
            let y = std::path::Path::new(r"C:\tmp\deck-spike3\y.txt");
            println!("y.txt exists={} content={:?}", y.exists(), std::fs::read_to_string(y).ok());
            let q = s.quit();
            println!("{} quit exited={} in {:.2}s", ts(), q.0, q.1);
            std::thread::sleep(Duration::from_secs(2));
            show_new(shown, "end");
        }
        "c" => {
            let s = spawn(vec!["--session-id".into(), x.clone(), "--permission-mode".into(), "default".into()], t0);
            let st = wait_start(&s, ll0, 40);
            println!("{} C SessionStart: {}", ts(), st.is_some());
            if st.is_none() {
                println!("BLOCKED screen:\n{}", s.screen());
                let _ = s.pty.kill();
                return;
            }
            std::thread::sleep(Duration::from_secs(3));
            let ll = lines().len();
            s.w(b"/clear");
            std::thread::sleep(Duration::from_millis(500));
            s.w(b"\r");
            let st2 = wait_hook("SessionStart", ll, 20, "\"source\":\"clear\"");
            println!("{} C clear SessionStart: {:?}", ts(), st2);
            let y = st2.as_deref().map(session_id_of).unwrap_or_default();
            println!("new id Y = {y}");
            std::thread::sleep(Duration::from_secs(3));
            show_new(ll, "clear");
            jsonl_report(&y);
            let q = s.quit();
            println!("{} quit exited={} in {:.2}s code {:?}", ts(), q.0, q.1, s.exit_code());
            std::thread::sleep(Duration::from_secs(2));
            jsonl_report(&y);
            jsonl_report(&x);
            probe("C resume-Y", vec!["--resume".into(), y.clone()]);
        }
        "d" => {
            let id = std::env::args().nth(2).unwrap();
            probe("D session-id-reuse-with-transcript", vec!["--session-id".into(), id]);
        }
        _ => println!("unknown scenario"),
    }
}
