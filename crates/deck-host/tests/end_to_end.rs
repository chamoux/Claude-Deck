//! End-to-end tests: the real helper (`Core` + `runtime::run`) in this process, a window talking
//! to it over the real named pipe, and `fake-claude` standing in for Claude.
//!
//! Pipe name: each test listens on a unique pipe. `CoreConfig.pipe_name` passes it to Claude as
//! `CLAUDE_DECK_PIPE`, and the real `deck-host hook` binary connects there. Tests run in parallel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deck_core::state::SessionState;
use deck_core::store::{Project, SavedState, Settings};
use deck_host::framed::{read_frame, write_frame};
use deck_host::host::core::{Core, CoreConfig};
use deck_host::host::runtime::{run, RuntimeConfig, DEFAULT_MAX_WINDOW_QUEUE};
use deck_host::pipe::{connect_client, current_user_sid, PipeSecurity, SingleInstance};
use deck_proto::messages::{
    decode_event, encode_request, ClientKind, Event, Request, SessionView, Target,
};
use deck_proto::{Frame, FrameDecoder};
use tokio::io::{split, WriteHalf};
use tokio::net::windows::named_pipe::NamedPipeClient;
use tokio::task::JoinHandle;

const T: Duration = Duration::from_secs(15);

static NEXT: AtomicU32 = AtomicU32::new(0);

// ---- window client ---------------------------------------------------------

#[derive(Default)]
struct Shared {
    events: Vec<Event>,
    outputs: HashMap<u32, Vec<u8>>,
    sessions: HashMap<String, SessionView>,
    projects: Vec<Project>,
}

struct WindowClient {
    shared: Arc<Mutex<Shared>>,
    writer: tokio::sync::Mutex<WriteHalf<NamedPipeClient>>,
    cursor: Mutex<usize>,
    reader: JoinHandle<()>,
}

impl Drop for WindowClient {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

async fn connect_retry(name: &str) -> NamedPipeClient {
    let deadline = Instant::now() + T;
    loop {
        match connect_client(name, Duration::from_secs(3)).await {
            Ok(p) => return p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(e) => panic!("connect failed: {e}"),
        }
    }
}

impl WindowClient {
    async fn connect(name: &str) -> Self {
        let pipe = connect_retry(name).await;
        let (mut rd, wr) = split(pipe);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let sh = shared.clone();
        let reader = tokio::spawn(async move {
            let mut dec = FrameDecoder::new();
            while let Ok(Some(frame)) = read_frame(&mut rd, &mut dec).await {
                let mut s = sh.lock().unwrap();
                match frame {
                    Frame::Output { session, data } => s
                        .outputs
                        .entry(session)
                        .or_default()
                        .extend_from_slice(&data),
                    Frame::Control(json) => {
                        let Ok(ev) = decode_event(&json) else {
                            continue;
                        };
                        match &ev {
                            Event::State { state } => {
                                s.projects = state.projects.clone();
                                s.sessions = state
                                    .sessions
                                    .iter()
                                    .map(|v| (v.deck_id.clone(), v.clone()))
                                    .collect();
                            }
                            Event::SessionChanged { session } => {
                                s.sessions.insert(session.deck_id.clone(), session.clone());
                            }
                            _ => {}
                        }
                        s.events.push(ev);
                    }
                    Frame::Input { .. } => {}
                }
            }
        });
        let c = Self {
            shared,
            writer: tokio::sync::Mutex::new(wr),
            cursor: Mutex::new(0),
            reader,
        };
        c.send(Request::Hello {
            client: ClientKind::Window,
        })
        .await;
        c
    }

    async fn send(&self, r: Request) {
        let mut w = self.writer.lock().await;
        write_frame(&mut *w, &encode_request(&r)).await.unwrap();
    }

    /// Sends keyboard input to the terminal with this index.
    async fn type_line(&self, index: u32, text: &str) {
        let mut w = self.writer.lock().await;
        write_frame(
            &mut *w,
            &Frame::Input {
                session: index,
                data: text.as_bytes().to_vec(),
            },
        )
        .await
        .unwrap();
    }

    /// Next event (from the internal cursor on) that satisfies `pred`; moves the cursor past it.
    async fn wait_event(
        &self,
        pred: impl Fn(&Event) -> bool,
        what: &str,
        timeout: Duration,
    ) -> Event {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let s = self.shared.lock().unwrap();
                let mut cur = self.cursor.lock().unwrap();
                if let Some(pos) = s.events[*cur..].iter().position(&pred) {
                    let ev = s.events[*cur + pos].clone();
                    *cur += pos + 1;
                    return ev;
                }
            }
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The next time (after the cursor) this session is reported in `state`.
    async fn wait_session_state(&self, deck_id: &str, state: SessionState, timeout: Duration) {
        let id = deck_id.to_string();
        self.wait_event(
            move |e| match e {
                Event::SessionChanged { session } => {
                    session.deck_id == id && session.state == state
                }
                Event::State { state: st } => st
                    .sessions
                    .iter()
                    .any(|v| v.deck_id == id && v.state == state),
                _ => false,
            },
            &format!("session {deck_id} -> {state:?}"),
            timeout,
        )
        .await;
    }

    /// Waits until the latest known state of the session is `state`; ignores older events.
    async fn wait_now(&self, deck_id: &str, state: SessionState) {
        self.wait_cond(&format!("session {deck_id} now {state:?}"), || {
            self.session(deck_id).map(|s| s.state) == Some(state)
        })
        .await;
    }

    async fn wait_cond(&self, what: &str, cond: impl Fn() -> bool) {
        let deadline = Instant::now() + T;
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn session(&self, deck_id: &str) -> Option<SessionView> {
        self.shared.lock().unwrap().sessions.get(deck_id).cloned()
    }

    fn outputs_for(&self, index: u32) -> String {
        let s = self.shared.lock().unwrap();
        String::from_utf8_lossy(s.outputs.get(&index).map(Vec::as_slice).unwrap_or(&[]))
            .into_owned()
    }

    async fn attach(&self, target: Target) -> u32 {
        self.send(Request::Attach {
            target: target.clone(),
            rows: 40,
            cols: 120,
        })
        .await;
        let t = target.clone();
        match self
            .wait_event(
                move |e| matches!(e, Event::Attached { target, .. } if *target == t),
                "Attached",
                T,
            )
            .await
        {
            Event::Attached { index, .. } => index,
            _ => unreachable!(),
        }
    }

    async fn add_project(&self, path: &Path) -> String {
        let p = path.to_string_lossy().to_string();
        self.send(Request::AddProject { path: p.clone() }).await;
        let deadline = Instant::now() + T;
        loop {
            if let Some(pr) = self
                .shared
                .lock()
                .unwrap()
                .projects
                .iter()
                .find(|x| x.path == p)
            {
                return pr.id.clone();
            }
            assert!(Instant::now() < deadline, "project never appeared");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn new_session(&self, project_id: &str) -> String {
        let known: Vec<String> = self
            .shared
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect();
        self.send(Request::NewSession {
            project_id: project_id.to_string(),
        })
        .await;
        let deadline = Instant::now() + T;
        loop {
            {
                let s = self.shared.lock().unwrap();
                if let Some(id) = s.sessions.keys().find(|k| !known.contains(k)) {
                    return id.clone();
                }
            }
            assert!(Instant::now() < deadline, "new session never appeared");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

// ---- harness ---------------------------------------------------------------

struct Harness {
    win: WindowClient,
    pipe: String,
    dir: tempfile::TempDir,
    clock: Arc<AtomicI64>,
    host: JoinHandle<std::io::Result<()>>,
    projects: Vec<tempfile::TempDir>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

struct Opts {
    idle_exit_ms: i64,
    max_window_queue_bytes: usize,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            idle_exit_ms: 60_000,
            max_window_queue_bytes: DEFAULT_MAX_WINDOW_QUEUE,
        }
    }
}

fn unique_pipe() -> String {
    format!(
        r"\\.\pipe\claude-deck-e2e-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    )
}

/// Starts a helper (runtime) on `dir`'s state.json with `state` loaded. Returns its task and the
/// clock offset it runs on.
fn start_host(
    dir: &Path,
    pipe: &str,
    state: SavedState,
    opts: &Opts,
) -> (JoinHandle<std::io::Result<()>>, Arc<AtomicI64>) {
    let sid = current_user_sid().unwrap();
    let hooks_file = dir.join("hooks.json");
    deck_host::hook_cmd::write_hooks_file(&hooks_file, Path::new(env!("CARGO_BIN_EXE_deck-host")))
        .unwrap();
    let core = Core::new(
        state,
        CoreConfig {
            claude_exe: PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")),
            hooks_file,
            shell: ("cmd.exe".into(), vec![]),
            ctrl_c_gap_ms: 300,
            sleep_grace_ms: 3000,
            idle_exit_ms: opts.idle_exit_ms,
            memory_every_ms: 5000,
            pipe_name: Some(pipe.to_string()),
        },
    );
    let clock = Arc::new(AtomicI64::new(0)); // offset added to the real time
    let c = clock.clone();
    let cfg = RuntimeConfig {
        pipe_name: pipe.to_string(),
        security: Arc::new(PipeSecurity::only(&sid).unwrap()),
        state_path: dir.join("state.json"),
        claude_projects_dir: dir.join("projects"),
        tick_ms: 250,
        clock: Arc::new(move || now_ms() + c.load(Ordering::SeqCst)),
        max_window_queue_bytes: opts.max_window_queue_bytes,
    };
    (tokio::spawn(run(core, cfg)), clock)
}

async fn setup() -> Option<Harness> {
    setup_with(Opts::default()).await
}

async fn setup_with(opts: Opts) -> Option<Harness> {
    let pipe = unique_pipe();
    let dir = tempfile::tempdir().unwrap();
    let (host, clock) = start_host(dir.path(), &pipe, SavedState::default(), &opts);
    let win = WindowClient::connect(&pipe).await;
    Some(Harness {
        win,
        pipe,
        dir,
        clock,
        host,
        projects: vec![],
    })
}

impl Harness {
    fn advance_minutes(&self, m: i64) {
        self.clock.fetch_add(m * 60_000, Ordering::SeqCst);
    }

    /// A new project plus one session; waits until the session is Idle.
    async fn project_with_session(&mut self) -> (String, String) {
        let p = tempfile::tempdir().unwrap();
        let pid = self.win.add_project(p.path()).await;
        self.projects.push(p);
        let id = self.win.new_session(&pid).await;
        self.win.wait_now(&id, SessionState::Idle).await;
        (pid, id)
    }

    async fn extra_session(&self, pid: &str) -> String {
        let id = self.win.new_session(pid).await;
        self.win.wait_now(&id, SessionState::Idle).await;
        id
    }

    /// Attaches a session and sends a throwaway line first. fake-claude gets two cursor
    /// reports (ConPTY's own startup query and its own), and the stray one would otherwise
    /// prefix the first real command line.
    async fn attach_session(&self, deck_id: &str) -> u32 {
        let idx = self
            .win
            .attach(Target::Session {
                deck_id: deck_id.to_string(),
            })
            .await;
        self.win.type_line(idx, "noop\r").await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        idx
    }

    fn conversation_id(&self, deck_id: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.dir.path().join("state.json")).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        v["sessions"]
            .as_array()?
            .iter()
            .find(|s| s["id"] == deck_id)
            .and_then(|s| s["conversation_id"].as_str().map(str::to_string))
    }

    async fn finish(self) {
        self.win
            .send(Request::WindowClosing {
                keep_running: false,
            })
            .await;
        let ids: Vec<String> = self
            .win
            .shared
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.win.wait_now(&id, SessionState::Asleep).await;
        }
        self.host.abort();
    }
}

macro_rules! setup {
    () => {
        match setup().await {
            Some(h) => h,
            None => return,
        }
    };
}

// ---- scenarios -------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s1_new_session_idle_then_work() {
    let mut h = setup!();
    let (_pid, id) = h.project_with_session().await;
    let idx = h.attach_session(&id).await;
    h.win.type_line(idx, "work 3\r").await;
    tokio::time::sleep(Duration::from_secs(4)).await;
    h.win
        .wait_session_state(&id, SessionState::Working, T)
        .await;
    h.win.wait_session_state(&id, SessionState::Idle, T).await;
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s2_s3_auto_sleep_then_wake_resumes_same_conversation() {
    let mut h = setup!();
    let (_pid, id) = h.project_with_session().await;
    tokio::time::sleep(Duration::from_millis(1500)).await; // let a save happen
    let before = h.conversation_id(&id);
    h.win
        .send(Request::SetSettings {
            settings: Settings {
                auto_sleep_minutes: Some(5),
                ..Settings::default()
            },
        })
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.advance_minutes(6);
    h.win.wait_now(&id, SessionState::Asleep).await;

    h.win
        .send(Request::Wake {
            deck_id: id.clone(),
        })
        .await;
    h.win.wait_now(&id, SessionState::Idle).await;
    tokio::time::sleep(Duration::from_millis(1500)).await; // let a save happen
    let after = h.conversation_id(&id);
    assert!(before.is_some() && after.is_some());
    assert_eq!(before, after, "wake must keep the conversation id");
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s4_background_work_never_auto_sleeps() {
    let mut h = setup!();
    let (_pid, id) = h.project_with_session().await;
    h.win
        .send(Request::SetSettings {
            settings: Settings {
                auto_sleep_minutes: Some(5),
                ..Settings::default()
            },
        })
        .await;
    let idx = h.attach_session(&id).await;
    h.win.type_line(idx, "background\r").await;
    h.win
        .wait_session_state(&id, SessionState::Working, T)
        .await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    h.advance_minutes(60);
    tokio::time::sleep(Duration::from_millis(1500)).await; // several ticks
    assert_eq!(
        h.win.session(&id).map(|s| s.state),
        Some(SessionState::Working)
    );
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s5_ask_needs_you_then_answer() {
    let mut h = setup!();
    let (_pid, id) = h.project_with_session().await;
    let idx = h.attach_session(&id).await;
    h.win.type_line(idx, "ask\r").await;
    h.win
        .wait_session_state(&id, SessionState::NeedsYou, T)
        .await;
    h.win.type_line(idx, "y\r").await;
    h.win
        .wait_session_state(&id, SessionState::Working, T)
        .await;
    h.win.wait_session_state(&id, SessionState::Idle, T).await;
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s6_clear_changes_conversation_id() {
    let mut h = setup!();
    let (_pid, id) = h.project_with_session().await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let before = h.conversation_id(&id).expect("conversation id saved");
    let idx = h.attach_session(&id).await;
    h.win.type_line(idx, "clear\r").await;
    let deadline = Instant::now() + T;
    loop {
        if h.conversation_id(&id).is_some_and(|c| c != before) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "conversation id never changed after /clear"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    h.win.wait_now(&id, SessionState::Idle).await;
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s7_attach_snapshot_live_output_and_detach() {
    let mut h = setup!();
    let (_pid, id) = h.project_with_session().await;
    let target = Target::Session {
        deck_id: id.clone(),
    };
    let idx = h.win.attach(target.clone()).await;
    h.win
        .wait_cond("snapshot with fake-claude ready", || {
            h.win.outputs_for(idx).contains("fake-claude ready")
        })
        .await;
    h.win.type_line(idx, "noop\r").await; // absorbs the stray cursor report, see attach_session
    tokio::time::sleep(Duration::from_millis(400)).await;
    let before = h.win.outputs_for(idx).len();
    h.win.type_line(idx, "work 1\r").await;
    h.win
        .wait_session_state(&id, SessionState::Working, T)
        .await;
    h.win.wait_session_state(&id, SessionState::Idle, T).await;
    h.win
        .wait_cond("live dots", || {
            let o = h.win.outputs_for(idx);
            o[before.min(o.len())..].contains('.')
        })
        .await;

    h.win
        .send(Request::Detach {
            target: target.clone(),
        })
        .await;
    h.win
        .wait_event(|e| matches!(e, Event::Detached { .. }), "Detached", T)
        .await;
    let frozen = h.win.outputs_for(idx).len();

    // A second window drives the session; the detached first window must see nothing.
    let other = WindowClient::connect(&h.pipe).await;
    let idx2 = other.attach(target).await;
    other.type_line(idx2, "work 2\r").await;
    other
        .wait_session_state(&id, SessionState::Working, T)
        .await;
    other.wait_session_state(&id, SessionState::Idle, T).await;
    other
        .wait_cond("second window sees its own output", || {
            other.outputs_for(idx2).contains("work 2")
        })
        .await;
    assert_eq!(
        h.win.outputs_for(idx).len(),
        frozen,
        "output arrived after Detach"
    );
    drop(other);
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s8_project_shell() {
    let mut h = setup!();
    let (pid, _id) = h.project_with_session().await;
    let idx = h
        .win
        .attach(Target::Shell {
            project_id: pid.clone(),
        })
        .await;
    h.win
        .wait_cond("shell running", || {
            h.win
                .shared
                .lock()
                .unwrap()
                .events
                .iter()
                .any(|e| matches!(e, Event::ShellChanged { shell } if shell.running))
        })
        .await;
    h.win.type_line(idx, "echo hi-deck\r").await;
    h.win
        .wait_cond("shell echo output", || {
            h.win.outputs_for(idx).contains("hi-deck")
        })
        .await;
    h.win.send(Request::CloseShell { project_id: pid }).await;
    h.win
        .wait_event(
            |e| matches!(e, Event::ShellChanged { shell } if !shell.running),
            "shell stopped",
            T,
        )
        .await;
    h.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s9_window_closing_keep_running() {
    let mut h = setup!();
    let (pid, busy) = h.project_with_session().await;
    let idle = h.extra_session(&pid).await;
    let idx = h.attach_session(&busy).await;
    h.win.type_line(idx, "work 300\r").await;
    h.win
        .wait_session_state(&busy, SessionState::Working, T)
        .await;

    h.win
        .send(Request::WindowClosing { keep_running: true })
        .await;
    h.win.wait_now(&idle, SessionState::Asleep).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        h.win.session(&busy).map(|s| s.state),
        Some(SessionState::Working)
    );

    h.win
        .send(Request::WindowClosing {
            keep_running: false,
        })
        .await;
    h.win.wait_now(&busy, SessionState::Asleep).await;
    h.finish().await;
}

#[test]
fn s10_single_instance() {
    let sid = format!("S-1-5-test-e2e-{}", std::process::id());
    let first = SingleInstance::acquire(&sid).unwrap();
    assert!(first.is_some());
    assert!(SingleInstance::acquire(&sid).unwrap().is_none());
    drop(first);
    assert!(SingleInstance::acquire(&sid).unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s11_helper_restart_brings_sessions_back_asleep_with_names() {
    let mut h = setup_with(Opts {
        idle_exit_ms: 1000,
        ..Opts::default()
    })
    .await
    .unwrap();
    let (pid, a) = h.project_with_session().await;
    let b = h.extra_session(&pid).await;
    for (id, name) in [(&a, "alpha"), (&b, "beta")] {
        h.win
            .send(Request::Rename {
                deck_id: id.clone(),
                name: name.to_string(),
            })
            .await;
    }
    h.win
        .wait_cond("both renamed", || {
            h.win.session(&a).and_then(|s| s.name).as_deref() == Some("alpha")
                && h.win.session(&b).and_then(|s| s.name).as_deref() == Some("beta")
        })
        .await;
    h.win
        .send(Request::WindowClosing {
            keep_running: false,
        })
        .await;
    h.win.wait_now(&a, SessionState::Asleep).await;
    h.win.wait_now(&b, SessionState::Asleep).await;

    // Stop the helper the normal way: no window and nothing awake, so it exits and saves.
    let Harness {
        win,
        dir,
        host,
        projects,
        ..
    } = h;
    drop(win);
    tokio::time::timeout(Duration::from_secs(10), host)
        .await
        .expect("first helper did not exit")
        .unwrap()
        .unwrap();

    let state_path = dir.path().join("state.json");
    let loaded = deck_core::store::load(&state_path, now_ms()).unwrap();
    let pipe = unique_pipe();
    let (host2, _clock) = start_host(dir.path(), &pipe, loaded.state, &Opts::default());
    let win2 = WindowClient::connect(&pipe).await;
    win2.wait_now(&a, SessionState::Asleep).await;
    win2.wait_now(&b, SessionState::Asleep).await;
    assert_eq!(
        win2.session(&a).and_then(|s| s.name).as_deref(),
        Some("alpha")
    );
    assert_eq!(
        win2.session(&b).and_then(|s| s.name).as_deref(),
        Some("beta")
    );
    host2.abort();
    drop(projects);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s12_idle_exit_with_a_shell_running() {
    let idle_exit_ms = 1500;
    let mut h = setup_with(Opts {
        idle_exit_ms,
        ..Opts::default()
    })
    .await
    .unwrap();
    // A project without sessions: the shell is the only thing running.
    let p = tempfile::tempdir().unwrap();
    let pid = h.win.add_project(p.path()).await;
    h.projects.push(p);
    h.win
        .attach(Target::Shell {
            project_id: pid.clone(),
        })
        .await;
    h.win
        .wait_cond("shell running", || {
            h.win
                .shared
                .lock()
                .unwrap()
                .events
                .iter()
                .any(|e| matches!(e, Event::ShellChanged { shell } if shell.running))
        })
        .await;

    let Harness { win, host, .. } = h;
    drop(win);
    let closed = Instant::now();
    tokio::time::timeout(Duration::from_millis(idle_exit_ms as u64 + 5000), host)
        .await
        .expect("helper kept running with only a shell and no window")
        .unwrap()
        .unwrap();
    assert!(closed.elapsed() >= Duration::from_millis(idle_exit_ms as u64 - 500));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s13_window_that_never_reads_is_dropped_and_actor_stays_responsive() {
    const CAP: usize = 64 * 1024;
    let mut h = setup_with(Opts {
        max_window_queue_bytes: CAP,
        ..Opts::default()
    })
    .await
    .unwrap();
    let p = tempfile::tempdir().unwrap();
    let pid = h.win.add_project(p.path()).await;
    h.projects.push(p);

    // The bad window: Hello, attach a shell, learn its index, then never read again.
    let pipe = connect_retry(&h.pipe).await;
    let (mut rd, mut wr) = split(pipe);
    write_frame(
        &mut wr,
        &encode_request(&Request::Hello {
            client: ClientKind::Window,
        }),
    )
    .await
    .unwrap();
    write_frame(
        &mut wr,
        &encode_request(&Request::Attach {
            target: Target::Shell {
                project_id: pid.clone(),
            },
            rows: 40,
            cols: 120,
        }),
    )
    .await
    .unwrap();
    let mut dec = FrameDecoder::new();
    let index = tokio::time::timeout(T, async {
        loop {
            let frame = read_frame(&mut rd, &mut dec)
                .await
                .unwrap()
                .expect("closed");
            if let Frame::Control(json) = frame {
                if let Ok(Event::Attached { index, .. }) = decode_event(&json) {
                    return index;
                }
            }
        }
    })
    .await
    .expect("bad window never got Attached");
    // A flood of output (ping -t is far too slow to reach any cap quickly).
    write_frame(
        &mut wr,
        &Frame::Input {
            session: index,
            data: b"dir /s /b C:\\Windows\\System32\r".to_vec(),
        },
    )
    .await
    .unwrap();

    // Give the flood time to pass the cap while nobody reads.
    tokio::time::sleep(Duration::from_secs(4)).await;

    // The helper's actor must still answer another window.
    let states_before = h
        .win
        .shared
        .lock()
        .unwrap()
        .events
        .iter()
        .filter(|e| matches!(e, Event::State { .. }))
        .count();
    h.win.send(Request::GetState).await;
    h.win
        .wait_cond("GetState answered", || {
            h.win
                .shared
                .lock()
                .unwrap()
                .events
                .iter()
                .filter(|e| matches!(e, Event::State { .. }))
                .count()
                > states_before
        })
        .await;

    // Now read: what is already in the pipe arrives, then the helper has closed the connection.
    tokio::time::timeout(T, async {
        loop {
            match read_frame(&mut rd, &mut dec).await {
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return,
            }
        }
    })
    .await
    .expect("slow window was never disconnected");

    h.win.send(Request::CloseShell { project_id: pid }).await;
    h.finish().await;
}
