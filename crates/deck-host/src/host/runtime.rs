//! Tokio actor: pipe server, PTY events, timers, effects.
//!
//! One actor owns `Core`. Every other task talks to it through one unbounded channel of
//! `Msg`, so a client's `Register` always reaches the actor before that client's requests.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use deck_core::store::SavedState;
use deck_proto::messages::{decode_request, ClientKind, Request, Target};
use deck_proto::{Frame, FrameDecoder, HOOK_ACK};
use tokio::io::{split, AsyncWriteExt};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::{mpsc, oneshot, watch, Notify};

use crate::framed::{read_frame, write_frame};
use crate::git_runner::git_status;
use crate::host::core::{ClientId, Core, Effect, Input};
use crate::memory::MemoryMeter;
use crate::pipe::{create_server, PipeSecurity};
use crate::pty::{PtyEvent, PtyProcess};

pub struct RuntimeConfig {
    pub pipe_name: String,
    pub security: Arc<PipeSecurity>,
    pub state_path: PathBuf,
    /// `%USERPROFILE%\.claude\projects`
    pub claude_projects_dir: PathBuf,
    pub tick_ms: u64,
    /// Current time in ms (Unix ms in production).
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// A window with more than this many bytes queued (not yet written to its pipe) is dropped.
    /// It reconnects and re-attaches with fresh snapshots. Default: [`DEFAULT_MAX_WINDOW_QUEUE`].
    pub max_window_queue_bytes: usize,
}

/// Default for `RuntimeConfig::max_window_queue_bytes`: 32 MiB.
pub const DEFAULT_MAX_WINDOW_QUEUE: usize = 32 * 1024 * 1024;
/// `Frame::Output` data larger than this is split into several frames with the same index.
const MAX_OUTPUT_CHUNK: usize = 1024 * 1024;
const LOG_KEEP: Duration = Duration::from_secs(7 * 24 * 3600);
const LOG_PURGE_EVERY: Duration = Duration::from_secs(24 * 3600);

const SAVE_EVERY: Duration = Duration::from_secs(1);
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const HOOK_ACK_TIMEOUT: Duration = Duration::from_secs(10);

// ---- logging ---------------------------------------------------------------

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Production clock: Unix milliseconds.
pub fn system_clock() -> Arc<dyn Fn() -> i64 + Send + Sync> {
    Arc::new(unix_ms)
}

/// Deletes `host-*` files in `dir` last modified more than `max_age` ago. Failures are ignored.
pub fn purge_old_logs(dir: &Path, max_age: Duration) {
    let cutoff = std::time::SystemTime::now() - max_age;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| t < cutoff)
                .unwrap_or(false);
            if old && e.file_name().to_string_lossy().starts_with("host-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Sets the log folder, creates it, and deletes `host-*` files older than 7 days. Failures are ignored.
pub fn init_logging(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    purge_old_logs(dir, LOG_KEEP);
    let _ = LOG_DIR.set(dir.to_path_buf());
    log("log started; all timestamps in this log are UTC");
}

/// `YYYY-MM-DD HH:MM:SS.mmm msg\n` (UTC), plus the date as `YYYYMMDD`.
fn stamp(msg: &str) -> (String, String) {
    let ms = unix_ms();
    let (y, mo, d) = civil_from_days(ms.div_euclid(86_400_000));
    let tod = ms.rem_euclid(86_400_000);
    let line = format!(
        "{y:04}-{mo:02}-{d:02} {:02}:{:02}:{:02}.{:03} {msg}\n",
        tod / 3_600_000,
        tod / 60_000 % 60,
        tod / 1000 % 60,
        tod % 1000
    );
    (line, format!("{y:04}{mo:02}{d:02}"))
}

fn append_line(file: &Path, line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Appends a timestamped line to `host-YYYYMMDD.log`. A no-op before `init_logging`.
pub fn log(msg: &str) {
    let Some(dir) = LOG_DIR.get() else { return };
    let (line, day) = stamp(msg);
    append_line(&dir.join(format!("host-{day}.log")), &line);
}

/// Installs a panic hook that passes `panic: {info}` to `sink`, then runs the previous hook.
pub fn install_panic_hook_with(sink: impl Fn(&str) + Send + Sync + 'static) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        sink(&format!("panic: {info}"));
        prev(info);
    }));
}

/// Panic hook that writes to the normal host log (see `init_logging`).
pub fn install_panic_hook() {
    install_panic_hook_with(log);
}

/// Panic hook that appends timestamped `panic:` lines to `path`.
pub fn install_panic_hook_to_file(path: PathBuf) {
    install_panic_hook_with(move |m| append_line(&path, &stamp(m).0));
}

// ---- actor messages --------------------------------------------------------

/// How the actor reaches a connected client.
enum Registration {
    Window(WindowHandle),
    Hook(oneshot::Sender<()>),
}

enum Msg {
    Input(Input),
    Register(ClientId, Registration),
    /// `generation` tells a stale exit from the exit of a newer process on the same target.
    PtyExited {
        target: Target,
        generation: u64,
        code: Option<u32>,
    },
}

fn frame_len(f: &Frame) -> usize {
    8 + match f {
        Frame::Control(d) => d.len(),
        Frame::Output { data, .. } | Frame::Input { data, .. } => data.len(),
    }
}

/// The actor's side of a window connection: an unbounded queue plus a byte counter, so one
/// slow window can be dropped instead of growing memory forever.
struct WindowHandle {
    tx: mpsc::UnboundedSender<Frame>,
    /// Bytes queued and not yet written; the writer task subtracts after each write.
    queued: Arc<AtomicUsize>,
    /// Tells the client task to close the connection.
    kick: Arc<Notify>,
}

impl WindowHandle {
    /// Queues a frame (splitting big output). `false` means the window is over `cap` or gone.
    fn push(&self, frame: Frame, cap: usize) -> bool {
        let frames = match frame {
            Frame::Output { session, data } if data.len() > MAX_OUTPUT_CHUNK => data
                .chunks(MAX_OUTPUT_CHUNK)
                .map(|c| Frame::Output {
                    session,
                    data: c.to_vec(),
                })
                .collect(),
            f => vec![f],
        };
        for f in frames {
            let n = frame_len(&f);
            if self.queued.fetch_add(n, Ordering::Relaxed) + n > cap {
                return false;
            }
            if self.tx.send(f).is_err() {
                return false;
            }
        }
        true
    }
}

type Tx = mpsc::UnboundedSender<Msg>;

static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);

// ---- pipe server and client tasks ------------------------------------------

async fn server_loop(
    name: String,
    sec: Arc<PipeSecurity>,
    mut server: NamedPipeServer,
    tx: Tx,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = stop.changed() => return,
            r = server.connect() => {
                if let Err(e) = r {
                    log(&format!("pipe connect failed: {e}"));
                }
                let connected = server;
                // Always have the next instance ready before serving this one.
                server = loop {
                    match create_server(&name, &sec, false) {
                        Ok(s) => break s,
                        Err(e) => {
                            log(&format!("pipe create failed, retrying: {e}"));
                            tokio::select! {
                                _ = stop.changed() => return,
                                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                            }
                        }
                    }
                };
                tokio::spawn(client_task(connected, tx.clone(), stop.clone()));
            }
        }
    }
}

async fn client_task(pipe: NamedPipeServer, tx: Tx, mut stop: watch::Receiver<bool>) {
    let id = NEXT_CLIENT.fetch_add(1, Ordering::Relaxed);
    tokio::select! {
        _ = stop.changed() => {}
        _ = serve_client(id, pipe, &tx) => {}
    }
}

async fn serve_client(id: ClientId, mut pipe: NamedPipeServer, tx: &Tx) {
    let mut dec = FrameDecoder::new();
    let hello = tokio::time::timeout(HELLO_TIMEOUT, read_frame(&mut pipe, &mut dec)).await;
    let kind = match hello {
        Ok(Ok(Some(Frame::Control(json)))) => match decode_request(&json) {
            Ok(Request::Hello { client }) => client,
            _ => return,
        },
        _ => return,
    };
    match kind {
        ClientKind::Hook => hook_client(id, pipe, dec, tx).await,
        ClientKind::Window => window_client(id, pipe, dec, tx).await,
    }
}

async fn hook_client(id: ClientId, mut pipe: NamedPipeServer, mut dec: FrameDecoder, tx: &Tx) {
    let (ack_tx, ack_rx) = oneshot::channel();
    let _ = tx.send(Msg::Register(id, Registration::Hook(ack_tx)));
    let req = match tokio::time::timeout(HELLO_TIMEOUT, read_frame(&mut pipe, &mut dec)).await {
        Ok(Ok(Some(Frame::Control(json)))) => match decode_request(&json) {
            Ok(r @ Request::HookEvent { .. }) => Some(r),
            _ => None,
        },
        _ => None,
    };
    if let Some(req) = req {
        let _ = tx.send(Msg::Input(Input::Request(id, req)));
        if let Ok(Ok(())) = tokio::time::timeout(HOOK_ACK_TIMEOUT, ack_rx).await {
            let _ = pipe.write_all(&[HOOK_ACK]).await;
            let _ = pipe.flush().await;
        }
    }
    // Hook clients are never windows; core ignores this id. It also frees the registration.
    let _ = tx.send(Msg::Input(Input::ClientGone(id)));
}

async fn window_client(id: ClientId, pipe: NamedPipeServer, mut dec: FrameDecoder, tx: &Tx) {
    let (mut rd, mut wr) = split(pipe);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Frame>();
    let queued = Arc::new(AtomicUsize::new(0));
    let kick = Arc::new(Notify::new());
    let _ = tx.send(Msg::Register(
        id,
        Registration::Window(WindowHandle {
            tx: out_tx,
            queued: queued.clone(),
            kick: kick.clone(),
        }),
    ));
    // Core answers WindowConnected with the State; the Hello itself is not forwarded.
    let _ = tx.send(Msg::Input(Input::WindowConnected(id)));
    let mut writer = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            let n = frame_len(&frame);
            if write_frame(&mut wr, &frame).await.is_err() {
                break;
            }
            queued.fetch_sub(n, Ordering::Relaxed);
        }
    });
    loop {
        tokio::select! {
            // The writer ended: a write failed, so this connection is dead.
            _ = &mut writer => break,
            _ = kick.notified() => {
                log(&format!("window {id}: dropped, too much unsent data"));
                break;
            }
            r = read_frame(&mut rd, &mut dec) => match r {
                Ok(Some(Frame::Control(json))) => match decode_request(&json) {
                    Ok(req) => {
                        let _ = tx.send(Msg::Input(Input::Request(id, req)));
                    }
                    Err(e) => log(&format!("client {id}: bad request: {e}")),
                },
                Ok(Some(Frame::Input { session, data })) => {
                    let _ = tx.send(Msg::Input(Input::TerminalInput {
                        index: session,
                        bytes: data,
                    }));
                }
                Ok(Some(Frame::Output { .. })) => {}
                Ok(None) | Err(_) => break,
            },
        }
    }
    let _ = tx.send(Msg::Input(Input::ClientGone(id)));
    writer.abort();
}

// ---- actor -------------------------------------------------------------------

struct Saver {
    path: PathBuf,
    pending: Option<SavedState>,
    inflight: Option<tokio::task::JoinHandle<()>>,
    last: Option<Instant>,
}

fn save_blocking(path: &Path, state: &SavedState) {
    if let Err(e) = deck_core::store::save(path, state) {
        log(&format!("save failed: {e}"));
    }
}

impl Saver {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            pending: None,
            inflight: None,
            last: None,
        }
    }

    /// Writes the latest pending state if the last write was over a second ago.
    fn maybe_flush(&mut self) {
        if self.pending.is_none()
            || self.inflight.as_ref().is_some_and(|h| !h.is_finished())
            || self.last.is_some_and(|t| t.elapsed() < SAVE_EVERY)
        {
            return;
        }
        if let Some(state) = self.pending.take() {
            self.last = Some(Instant::now());
            let path = self.path.clone();
            self.inflight = Some(tokio::task::spawn_blocking(move || {
                save_blocking(&path, &state)
            }));
        }
    }

    async fn flush_now(&mut self) {
        if let Some(h) = self.inflight.take() {
            let _ = h.await;
        }
        if let Some(state) = self.pending.take() {
            let path = self.path.clone();
            let _ = tokio::task::spawn_blocking(move || save_blocking(&path, &state)).await;
        }
    }
}

struct Actor {
    core: Core,
    cfg: RuntimeConfig,
    tx: Tx,
    windows: HashMap<ClientId, WindowHandle>,
    hooks: HashMap<ClientId, oneshot::Sender<()>>,
    ptys: HashMap<Target, (u64, Arc<PtyProcess>)>,
    next_generation: u64,
    meter: Arc<Mutex<MemoryMeter>>,
    saver: Saver,
}

impl Actor {
    fn feed(&mut self, input: Input) -> Vec<Effect> {
        let gone = match &input {
            Input::ClientGone(id) => Some(*id),
            _ => None,
        };
        let effects = self.core.handle(input, (self.cfg.clock)());
        if let Some(id) = gone {
            self.windows.remove(&id);
            self.hooks.remove(&id);
        }
        effects
    }

    /// Queues a frame for one window; a window over the byte cap is dropped.
    fn deliver(&mut self, client: ClientId, frame: Frame) {
        let cap = self.cfg.max_window_queue_bytes;
        let Some(w) = self.windows.get(&client) else {
            return;
        };
        if !w.push(frame, cap) {
            self.drop_window(client);
        }
    }

    fn drop_window(&mut self, client: ClientId) {
        if let Some(w) = self.windows.remove(&client) {
            // Over the cap, or the connection is already gone; kicking is harmless either way.
            w.kick.notify_one();
        }
    }

    /// Returns true when the actor must stop.
    fn run_effects(&mut self, effects: Vec<Effect>) -> bool {
        for e in effects {
            if matches!(e, Effect::Exit) {
                return true;
            }
            self.effect(e);
        }
        false
    }

    fn spawn_pty(&mut self, target: Target, spec: &crate::pty::SpawnSpec) {
        self.next_generation += 1;
        let generation = self.next_generation;
        let tx = self.tx.clone();
        let t = target.clone();
        let mode = if spec.args.iter().any(|a| a == "--resume") {
            "resume"
        } else if spec.args.iter().any(|a| a == "--session-id") {
            "session-id"
        } else {
            "plain"
        };
        log(&format!("spawn {target:?} ({mode})"));
        let result = PtyProcess::spawn(spec, move |ev| {
            let msg = match ev {
                PtyEvent::Output(bytes) => Msg::Input(Input::PtyOutput {
                    target: t.clone(),
                    bytes,
                }),
                PtyEvent::Exited(code) => Msg::PtyExited {
                    target: t.clone(),
                    generation,
                    code,
                },
            };
            let _ = tx.send(msg);
        });
        match result {
            Ok(p) => {
                self.ptys.insert(target, (generation, p));
            }
            Err(err) => {
                log(&format!("spawn failed for {target:?}: {err}"));
                let _ = self.tx.send(Msg::Input(Input::SpawnFailed {
                    target,
                    error: err.to_string(),
                }));
            }
        }
    }

    fn effect(&mut self, e: Effect) {
        match e {
            Effect::Spawn { target, spec } => self.spawn_pty(target, &spec),
            Effect::Write { target, bytes } => {
                if let Some((_, p)) = self.ptys.get(&target) {
                    if let Err(err) = p.write(&bytes) {
                        log(&format!("write to {target:?} failed: {err}"));
                    }
                }
            }
            Effect::Resize { target, rows, cols } => {
                if let Some((_, p)) = self.ptys.get(&target) {
                    if let Err(err) = p.resize(rows, cols) {
                        log(&format!("resize of {target:?} failed: {err}"));
                    }
                }
            }
            Effect::Kill { target } => {
                log(&format!("kill {target:?}"));
                if let Some((_, p)) = self.ptys.get(&target) {
                    if let Err(err) = p.kill() {
                        log(&format!("kill of {target:?} failed: {err}"));
                    }
                }
            }
            Effect::Send { client, frame } => {
                self.deliver(client, frame);
            }
            Effect::Broadcast { frame } => {
                let cap = self.cfg.max_window_queue_bytes;
                let over: Vec<ClientId> = self
                    .windows
                    .iter()
                    .filter(|(_, w)| !w.push(frame.clone(), cap))
                    .map(|(id, _)| *id)
                    .collect();
                for id in over {
                    self.drop_window(id);
                }
            }
            Effect::HookAck { client } => {
                if let Some(ack) = self.hooks.remove(&client) {
                    let _ = ack.send(());
                }
            }
            Effect::RunGit {
                client,
                project_id,
                path,
            } => {
                let tx = self.tx.clone();
                tokio::task::spawn_blocking(move || {
                    let result = git_status(&path);
                    let _ = tx.send(Msg::Input(Input::GitDone {
                        client,
                        project_id,
                        result,
                    }));
                });
            }
            Effect::ListPast {
                client,
                project_id,
                path,
                exclude,
            } => {
                let tx = self.tx.clone();
                let root = self.cfg.claude_projects_dir.clone();
                tokio::task::spawn_blocking(move || {
                    let sessions = deck_core::claude_dir::list_past_sessions(
                        &root,
                        &path.to_string_lossy(),
                        &exclude,
                    );
                    let _ = tx.send(Msg::Input(Input::PastDone {
                        client,
                        project_id,
                        sessions,
                    }));
                });
            }
            Effect::MeasureMemory { targets } => {
                let pids: Vec<u32> = targets
                    .iter()
                    .filter_map(|t| self.ptys.get(t).and_then(|(_, p)| p.pid()))
                    .collect();
                let awake = targets.len() as u32;
                let meter = self.meter.clone();
                let tx = self.tx.clone();
                tokio::task::spawn_blocking(move || {
                    let bytes = match meter.lock() {
                        Ok(mut m) => m.private_bytes(&pids),
                        Err(_) => 0,
                    };
                    let _ = tx.send(Msg::Input(Input::MemoryMeasured { awake, bytes }));
                });
            }
            Effect::Save { state } => {
                self.saver.pending = Some(state);
                self.saver.maybe_flush();
            }
            Effect::Exit => {}
        }
    }

    /// Returns true when the actor must stop.
    fn handle(&mut self, msg: Msg) -> bool {
        let input = match msg {
            Msg::Register(id, Registration::Window(w)) => {
                self.windows.insert(id, w);
                return false;
            }
            Msg::Register(id, Registration::Hook(ack)) => {
                self.hooks.insert(id, ack);
                return false;
            }
            Msg::PtyExited {
                target,
                generation,
                code,
            } => {
                let current = self
                    .ptys
                    .get(&target)
                    .is_some_and(|(g, _)| *g == generation);
                if !current {
                    // An older process on this target; Core already moved on to a newer one.
                    log(&format!("stale exit of {target:?} ignored (code {code:?})"));
                    return false;
                }
                self.ptys.remove(&target);
                log(&format!("pty exited {target:?} code {code:?}"));
                Input::PtyExited { target }
            }
            Msg::Input(i) => i,
        };
        let is_tick = matches!(input, Input::Tick);
        let effects = self.feed(input);
        let exit = self.run_effects(effects);
        if is_tick {
            self.saver.maybe_flush();
        }
        exit
    }
}

/// Deletes old log files once a day while the helper runs (startup does its own purge).
async fn daily_log_purge(mut stop: watch::Receiver<bool>) {
    let mut iv = tokio::time::interval_at(
        tokio::time::Instant::now() + LOG_PURGE_EVERY,
        LOG_PURGE_EVERY,
    );
    loop {
        tokio::select! {
            _ = stop.changed() => return,
            _ = iv.tick() => {
                if let Some(dir) = LOG_DIR.get() {
                    let dir = dir.clone();
                    let _ = tokio::task::spawn_blocking(move || purge_old_logs(&dir, LOG_KEEP)).await;
                }
            }
        }
    }
}

/// Runs until Core emits `Effect::Exit`, then flushes any pending save and returns.
pub async fn run(core: Core, cfg: RuntimeConfig) -> std::io::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let (stop_tx, stop_rx) = watch::channel(false);

    let first = create_server(&cfg.pipe_name, &cfg.security, true)?;
    tokio::spawn(server_loop(
        cfg.pipe_name.clone(),
        cfg.security.clone(),
        first,
        tx.clone(),
        stop_rx.clone(),
    ));

    tokio::spawn(daily_log_purge(stop_rx.clone()));

    let tick_tx = tx.clone();
    let tick_ms = cfg.tick_ms.max(1);
    let mut tick_stop = stop_rx;
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(tick_ms));
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick_stop.changed() => return,
                _ = iv.tick() => {
                    if tick_tx.send(Msg::Input(Input::Tick)).is_err() {
                        return;
                    }
                }
            }
        }
    });

    let saver = Saver::new(cfg.state_path.clone());
    let mut actor = Actor {
        core,
        cfg,
        tx,
        windows: HashMap::new(),
        hooks: HashMap::new(),
        ptys: HashMap::new(),
        next_generation: 0,
        meter: Arc::new(Mutex::new(MemoryMeter::new())),
        saver,
    };
    while let Some(msg) = rx.recv().await {
        if actor.handle(msg) {
            break;
        }
    }
    actor.saver.flush_now().await;
    let _ = stop_tx.send(true);
    log("host exiting");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_in_spawned_thread_is_logged() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("panic.log");
        install_panic_hook_to_file(log_path.clone());
        let r = std::thread::spawn(|| panic!("boom-from-thread")).join();
        assert!(r.is_err());
        let text = std::fs::read_to_string(&log_path).unwrap();
        assert!(text.contains("panic: "), "{text}");
        assert!(text.contains("boom-from-thread"), "{text}");
    }

    #[test]
    fn purge_removes_only_old_host_logs() {
        let dir = tempfile::tempdir().unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
        let make = |name: &str, t: std::time::SystemTime| {
            let f = std::fs::File::create(dir.path().join(name)).unwrap();
            f.set_modified(t).unwrap();
        };
        make("host-20200101.log", old);
        make("host-new.log", std::time::SystemTime::now());
        make("other-old.txt", old);
        purge_old_logs(dir.path(), LOG_KEEP);
        assert!(!dir.path().join("host-20200101.log").exists());
        assert!(dir.path().join("host-new.log").exists());
        assert!(dir.path().join("other-old.txt").exists());
    }

    #[test]
    fn big_output_is_split_and_queue_cap_enforced() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let w = WindowHandle {
            tx,
            queued: Arc::new(AtomicUsize::new(0)),
            kick: Arc::new(Notify::new()),
        };
        let data = vec![7u8; 2 * MAX_OUTPUT_CHUNK + 5];
        assert!(w.push(Frame::Output { session: 3, data }, usize::MAX));
        let mut sizes = vec![];
        while let Ok(Frame::Output { session, data }) = rx.try_recv() {
            assert_eq!(session, 3);
            sizes.push(data.len());
        }
        assert_eq!(sizes, vec![MAX_OUTPUT_CHUNK, MAX_OUTPUT_CHUNK, 5]);
        assert!(!w.push(
            Frame::Control(vec![0; 100]),
            w.queued.load(Ordering::Relaxed) + 50
        ));
    }
}
