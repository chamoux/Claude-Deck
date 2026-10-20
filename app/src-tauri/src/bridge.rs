//! Bridge between the Tauri window and the `deck-host` helper over the named pipe.

use deck_host::framed::{read_frame, write_frame};
use deck_host::pipe::{connect_trusted, current_user_sid, pipe_name};
use deck_proto::messages::{encode_request, ClientKind, Request};
use deck_proto::{Frame, FrameDecoder};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::split;
use tokio::sync::{mpsc, oneshot};

/// Output buffered while no Channel is subscribed is capped at this many bytes.
pub const OUTPUT_BUFFER_CAP: usize = 4 * 1024 * 1024;

#[derive(Serialize, Clone, Debug)]
pub struct OutputChunk {
    pub index: u32,
    pub data: Vec<u8>,
}

/// Where routed frames end up.
pub trait Sink {
    fn event(&self, json: String);
    fn output(&self, index: u32, data: Vec<u8>);
}

/// Routes one helper frame: Control to `event`, Output to `output`, the rest is ignored.
pub fn route_frame(frame: Frame, sink: &impl Sink) {
    match frame {
        Frame::Control(json) => {
            if let Ok(s) = String::from_utf8(json) {
                sink.event(s);
            }
        }
        Frame::Output { session, data } => sink.output(session, data),
        Frame::Input { .. } => {}
    }
}

pub type ChunkSender = Box<dyn Fn(OutputChunk) -> bool + Send>;

#[derive(Default)]
struct HubInner {
    sender: Option<ChunkSender>,
    buf: VecDeque<(u32, Vec<u8>)>,
    size: usize,
}

/// Delivers output to the current subscriber, or buffers it (oldest dropped first).
pub struct OutputHub {
    inner: Mutex<HubInner>,
    cap: usize,
}

impl OutputHub {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(HubInner::default()),
            cap,
        }
    }

    pub fn push(&self, index: u32, data: Vec<u8>) {
        let mut g = self.inner.lock().unwrap();
        if let Some(send) = &g.sender {
            if send(OutputChunk {
                index,
                data: data.clone(),
            }) {
                return;
            }
            g.sender = None; // dead channel: buffer until the next subscribe
        }
        g.size += data.len();
        g.buf.push_back((index, data));
        while g.size > self.cap {
            let over = g.size - self.cap;
            let front_len = g.buf.front().map_or(0, |(_, d)| d.len());
            if front_len <= over {
                g.buf.pop_front();
                g.size -= front_len;
            } else {
                if let Some((_, d)) = g.buf.front_mut() {
                    d.drain(..over);
                }
                g.size -= over;
            }
        }
    }

    /// Replaces the subscriber and flushes whatever was buffered.
    pub fn subscribe(&self, send: ChunkSender) {
        let mut g = self.inner.lock().unwrap();
        while let Some((index, data)) = g.buf.pop_front() {
            g.size -= data.len();
            if !send(OutputChunk {
                index,
                data: data.clone(),
            }) {
                g.size += data.len();
                g.buf.push_front((index, data));
                g.sender = None;
                return;
            }
        }
        g.sender = Some(send);
    }
}

/// Whether a frame may wait in the queue while the helper is unreachable.
/// Input and Attach/Detach/Resize refer to terminal indices that a helper restart reassigns.
pub fn keep_while_disconnected(frame: &Frame) -> bool {
    match frame {
        Frame::Input { .. } | Frame::Output { .. } => false,
        Frame::Control(json) => !matches!(
            serde_json::from_slice::<Request>(json),
            Ok(Request::Attach { .. } | Request::Detach { .. } | Request::Resize { .. })
        ),
    }
}

pub struct Outgoing {
    frame: Frame,
    done: Option<oneshot::Sender<()>>,
}

/// Managed state shared with the commands.
pub struct Bridge {
    tx: mpsc::UnboundedSender<Outgoing>,
    pub hub: Arc<OutputHub>,
    pub closing: AtomicBool,
    connected: AtomicBool,
}

impl Bridge {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Outgoing>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let b = Self {
            tx,
            hub: Arc::new(OutputHub::new(OUTPUT_BUFFER_CAP)),
            closing: AtomicBool::new(false),
            connected: AtomicBool::new(false),
        };
        (b, rx)
    }

    pub fn send(&self, frame: Frame) -> Result<(), String> {
        if !self.connected.load(Ordering::SeqCst) && !keep_while_disconnected(&frame) {
            return Ok(());
        }
        self.tx
            .send(Outgoing { frame, done: None })
            .map_err(|_| "bridge is not running".to_string())
    }

    /// Queues a frame and waits up to `wait` for it to be written.
    pub async fn send_flushed(&self, frame: Frame, wait: Duration) {
        let (done, rx) = oneshot::channel();
        if self
            .tx
            .send(Outgoing {
                frame,
                done: Some(done),
            })
            .is_ok()
        {
            let _ = tokio::time::timeout(wait, rx).await;
        }
    }
}

/// Where to look for the helper, in priority order.
pub fn helper_candidates(exe_dir: &Path, env: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut v = vec![
        exe_dir.join("deck-host.exe"),
        exe_dir.join("deck-host-x86_64-pc-windows-msvc.exe"),
    ];
    if let Some(p) = env("CLAUDE_DECK_HOST") {
        v.push(PathBuf::from(p));
    }
    v.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/deck-host.exe"));
    if let Some(t) = env("CARGO_TARGET_DIR") {
        v.push(PathBuf::from(t).join("debug").join("deck-host.exe"));
    }
    v
}

fn launch_helper() -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    for c in helper_candidates(&exe_dir, |k| std::env::var(k).ok()) {
        if !c.is_file() {
            continue;
        }
        let spawned = Command::new(&c)
            .arg("serve")
            .creation_flags(0x0800_0000 | 0x0000_0008)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if spawned.is_ok() {
            return true;
        }
    }
    false
}

type PipeClient = tokio::net::windows::named_pipe::NamedPipeClient;

/// Shown when the pipe exists but belongs to another account.
pub const FOREIGN_PIPE_MESSAGE: &str =
    "Claude Deck's pipe is owned by another account. Close other users' Claude Deck and restart.";

/// The `deck:event` payload for a foreign-owned pipe.
pub fn foreign_pipe_event() -> String {
    serde_json::json!({ "type": "error", "message": FOREIGN_PIPE_MESSAGE }).to_string()
}

/// How long to wait before trying again after finding a foreign-owned pipe.
const FOREIGN_RETRY: Duration = Duration::from_secs(30);

/// Connects; if the helper is not up, launches it and retries for up to 5 s.
/// A pipe owned by another account (PermissionDenied) is returned at once, never relaunched.
async fn connect_or_launch(name: &str, sid: &str) -> std::io::Result<PipeClient> {
    match connect_trusted(name, Duration::from_millis(500), sid).await {
        Ok(c) => return Ok(c),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(e),
        Err(_) => {}
    }
    launch_helper();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match connect_trusted(name, Duration::from_millis(200), sid).await {
            Ok(c) => return Ok(c),
            Err(e)
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    || Instant::now() >= deadline =>
            {
                return Err(e)
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

struct TauriSink {
    app: AppHandle,
    hub: Arc<OutputHub>,
}

impl Sink for TauriSink {
    fn event(&self, json: String) {
        let _ = self.app.emit("deck:event", json);
    }
    fn output(&self, index: u32, data: Vec<u8>) {
        self.hub.push(index, data);
    }
}

fn backoff(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_millis(500),
        1 => Duration::from_secs(1),
        2 => Duration::from_secs(2),
        3 => Duration::from_secs(4),
        _ => Duration::from_secs(5),
    }
}

/// Runs for the life of the app: connect, serve, reconnect.
pub async fn run(app: AppHandle, mut rx: mpsc::UnboundedReceiver<Outgoing>) {
    let sink = TauriSink {
        app: app.clone(),
        hub: app.state::<Bridge>().hub.clone(),
    };
    let Ok(sid) = current_user_sid() else {
        let _ = app.emit("deck:connection", "failed");
        return;
    };
    let name = pipe_name(&sid);
    let mut carry: Vec<Outgoing> = Vec::new();
    loop {
        // Connect with backoff; `reconnecting` is emitted after the first failed attempt.
        let started = Instant::now();
        let mut attempt = 0usize;
        let mut failed_sent = false;
        let client = loop {
            match connect_or_launch(&name, &sid).await {
                Ok(c) => break c,
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    let _ = app.emit("deck:connection", "failed");
                    let _ = app.emit("deck:event", foreign_pipe_event());
                    tokio::time::sleep(FOREIGN_RETRY).await;
                }
                Err(_) => {
                    if attempt == 0 {
                        let _ = app.emit("deck:connection", "reconnecting");
                    }
                    if !failed_sent && started.elapsed() >= Duration::from_secs(30) {
                        failed_sent = true;
                        let _ = app.emit("deck:connection", "failed");
                    }
                    tokio::time::sleep(backoff(attempt)).await;
                    attempt += 1;
                }
            }
        };
        // Drop anything stale that queued up while disconnected.
        while let Ok(o) = rx.try_recv() {
            carry.push(o);
        }
        carry.retain(|o| keep_while_disconnected(&o.frame));
        let (mut rd, mut wr) = split(client);
        let hello = encode_request(&Request::Hello {
            client: ClientKind::Window,
        });
        if write_frame(&mut wr, &hello).await.is_err() {
            let _ = app.emit("deck:connection", "reconnecting");
            continue;
        }
        let mut write_failed = false;
        for o in std::mem::take(&mut carry) {
            if write_frame(&mut wr, &o.frame).await.is_err() {
                write_failed = true;
                carry.push(o);
            }
        }
        if write_failed {
            let _ = app.emit("deck:connection", "reconnecting");
            continue;
        }
        app.state::<Bridge>()
            .connected
            .store(true, Ordering::SeqCst);
        let _ = app.emit("deck:connection", "connected");

        let mut dec = FrameDecoder::new();
        loop {
            tokio::select! {
                f = read_frame(&mut rd, &mut dec) => match f {
                    Ok(Some(frame)) => route_frame(frame, &sink),
                    _ => break,
                },
                out = rx.recv() => match out {
                    Some(Outgoing { frame, done }) => {
                        let ok = write_frame(&mut wr, &frame).await.is_ok();
                        if let Some(d) = done {
                            let _ = d.send(());
                        }
                        if !ok {
                            break;
                        }
                    }
                    None => return,
                },
            }
        }
        app.state::<Bridge>()
            .connected
            .store(false, Ordering::SeqCst);
        if app.state::<Bridge>().closing.load(Ordering::SeqCst) {
            return;
        }
        let _ = app.emit("deck:connection", "reconnecting");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        events: RefCell<Vec<String>>,
        outputs: RefCell<Vec<(u32, Vec<u8>)>>,
    }
    impl Sink for Fake {
        fn event(&self, json: String) {
            self.events.borrow_mut().push(json);
        }
        fn output(&self, index: u32, data: Vec<u8>) {
            self.outputs.borrow_mut().push((index, data));
        }
    }

    type Got = Arc<Mutex<Vec<(u32, Vec<u8>)>>>;

    fn collector(got: &Got) -> ChunkSender {
        let g = got.clone();
        Box::new(move |c| {
            g.lock().unwrap().push((c.index, c.data));
            true
        })
    }

    #[test]
    fn control_goes_to_event() {
        let f = Fake::default();
        route_frame(Frame::Control(br#"{"type":"x"}"#.to_vec()), &f);
        assert_eq!(f.events.borrow().as_slice(), [r#"{"type":"x"}"#]);
        assert!(f.outputs.borrow().is_empty());
    }

    #[test]
    fn output_goes_to_output_and_input_is_ignored() {
        let f = Fake::default();
        route_frame(
            Frame::Output {
                session: 3,
                data: vec![1, 2],
            },
            &f,
        );
        route_frame(
            Frame::Input {
                session: 3,
                data: vec![9],
            },
            &f,
        );
        assert_eq!(f.outputs.borrow().as_slice(), [(3, vec![1, 2])]);
        assert!(f.events.borrow().is_empty());
    }

    #[test]
    fn hub_buffers_without_channel_then_flushes() {
        let hub = OutputHub::new(100);
        hub.push(1, vec![1, 2]);
        hub.push(2, vec![3]);
        let got: Got = Arc::default();
        hub.subscribe(collector(&got));
        hub.push(1, vec![4]);
        assert_eq!(
            *got.lock().unwrap(),
            vec![(1, vec![1, 2]), (2, vec![3]), (1, vec![4])]
        );
    }

    #[test]
    fn hub_cap_drops_oldest_first() {
        let hub = OutputHub::new(5);
        hub.push(1, vec![1, 2, 3]);
        hub.push(2, vec![4, 5, 6]); // 6 bytes: trims one from the oldest
        let got: Got = Arc::default();
        hub.subscribe(collector(&got));
        assert_eq!(
            *got.lock().unwrap(),
            vec![(1, vec![2, 3]), (2, vec![4, 5, 6])]
        );
    }

    fn ctl(r: &Request) -> Frame {
        encode_request(r)
    }

    #[test]
    fn disconnected_filter_drops_input() {
        assert!(!keep_while_disconnected(&Frame::Input {
            session: 1,
            data: vec![b'y']
        }));
    }

    #[test]
    fn disconnected_filter_drops_attach_detach_resize() {
        use deck_proto::messages::Target;
        let t = || Target::Session {
            deck_id: "a".into(),
        };
        assert!(!keep_while_disconnected(&ctl(&Request::Attach {
            target: t(),
            rows: 1,
            cols: 2
        })));
        assert!(!keep_while_disconnected(&ctl(&Request::Detach {
            target: t()
        })));
        assert!(!keep_while_disconnected(&ctl(&Request::Resize {
            target: t(),
            rows: 1,
            cols: 2
        })));
    }

    #[test]
    fn disconnected_filter_keeps_new_session() {
        assert!(keep_while_disconnected(&ctl(&Request::NewSession {
            project_id: "p".into()
        })));
    }

    #[test]
    fn foreign_pipe_event_is_error_json() {
        let v: serde_json::Value = serde_json::from_str(&foreign_pipe_event()).unwrap();
        assert_eq!(v["type"], "error");
        assert_eq!(
            v["message"],
            "Claude Deck's pipe is owned by another account. Close other users' Claude Deck and restart."
        );
    }

    #[test]
    fn candidates_installed_layout() {
        let v = helper_candidates(Path::new("C:/App"), |_| None);
        assert_eq!(v[0], Path::new("C:/App").join("deck-host.exe"));
        assert_eq!(
            v[1],
            Path::new("C:/App").join("deck-host-x86_64-pc-windows-msvc.exe")
        );
    }

    #[test]
    fn candidates_dev_layout() {
        let v = helper_candidates(Path::new("C:/App"), |k| match k {
            "CLAUDE_DECK_HOST" => Some("D:/h/deck-host.exe".into()),
            "CARGO_TARGET_DIR" => Some("C:/t".into()),
            _ => None,
        });
        assert_eq!(v[2], PathBuf::from("D:/h/deck-host.exe"));
        assert!(v[3].ends_with("target/debug/deck-host.exe"));
        assert_eq!(v[4], Path::new("C:/t").join("debug").join("deck-host.exe"));
        assert_eq!(v.len(), 5);
    }
}
