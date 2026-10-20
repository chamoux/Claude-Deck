//! PtyProcess: spawn in ConPTY, reader thread, waiter thread, write, resize, kill.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Added to the inherited environment.
    pub env: Vec<(String, String)>,
    /// Removed from the inherited environment (e.g. Claude Code's own
    /// CLAUDE_CODE_CHILD_SESSION marker, which turns off transcript saving).
    pub env_remove: Vec<String>,
    pub rows: u16,
    pub cols: u16,
    /// Put the child in a job object so `kill()` ends its whole process tree and the
    /// kernel ends it if the helper dies. Off for shells: killing their tree would also
    /// close GUI apps the user started from them (`code .`).
    pub kill_tree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyEvent {
    Output(Vec<u8>),
    Exited(Option<u32>),
}

/// One child in a ConPTY. Events arrive on the callback from a single background
/// dispatcher thread, in order; `Exited` is always last.
pub struct PtyProcess {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Feeds the writer thread, so a large write never blocks the caller.
    writer: Mutex<mpsc::Sender<Vec<u8>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: Option<u32>,
    /// Holds the child and everything it starts. `None` if creating or assigning it failed.
    job: Option<KillJob>,
}

/// A job object that kills every process in it when its last handle closes. The kernel
/// closes the handle when the helper exits or crashes, so no descendant outlives it.
struct KillJob(windows::Win32::Foundation::HANDLE);

// SAFETY: a job handle is an opaque kernel handle, usable from any thread.
unsafe impl Send for KillJob {}
unsafe impl Sync for KillJob {}

fn win_err(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
}

impl KillJob {
    /// Creates the job and puts `process` in it. Children are created inside the job.
    /// No BREAKAWAY_OK: Git Bash (MSYS2) asks for CREATE_BREAKAWAY_FROM_JOB whenever the
    /// job allows it, so hook and shell children would escape.
    fn for_process(process: std::os::windows::io::RawHandle) -> io::Result<Self> {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        // SAFETY: plain Win32 calls. `info` outlives SetInformationJobObject. The job handle
        // is closed on every error path and otherwise owned by the returned KillJob.
        // `process` is the live child's handle, owned by portable-pty for this call.
        unsafe {
            let job = CreateJobObjectW(None, windows::core::PCWSTR::null()).map_err(win_err)?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            let assigned = set.and_then(|()| AssignProcessToJobObject(job, HANDLE(process)));
            if let Err(e) = assigned {
                let _ = CloseHandle(job);
                return Err(win_err(e));
            }
            Ok(Self(job))
        }
    }

    fn terminate(&self) -> io::Result<()> {
        // SAFETY: the handle is valid until Drop.
        unsafe { windows::Win32::System::JobObjects::TerminateJobObject(self.0, 1) }
            .map_err(win_err)
    }
}

impl Drop for KillJob {
    fn drop(&mut self) {
        // SAFETY: owned handle, closed exactly once. KILL_ON_JOB_CLOSE ends any leftovers.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

enum Msg {
    Output(Vec<u8>),
    ReaderDone,
    Exit(Option<u32>),
}

fn io_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

impl PtyProcess {
    pub fn spawn(
        spec: &SpawnSpec,
        mut on_event: impl FnMut(PtyEvent) + Send + 'static,
    ) -> io::Result<Arc<Self>> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io_err)?;
        let mut cmd = CommandBuilder::new(&spec.program);
        for a in &spec.args {
            cmd.arg(a);
        }
        cmd.cwd(&spec.cwd);
        for k in &spec.env_remove {
            cmd.env_remove(k);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).map_err(io_err)?;
        drop(pair.slave);
        // portable-pty does not start the child suspended; Claude and the shells take far
        // longer than this to start their own children, so the race is theoretical.
        let job = if spec.kill_tree {
            child
                .as_raw_handle()
                .and_then(|h| KillJob::for_process(h).ok())
        } else {
            None
        };
        let killer = child.clone_killer();
        let pid = child.process_id();
        let mut reader = pair.master.try_clone_reader().map_err(io_err)?;
        let mut writer = pair.master.take_writer().map_err(io_err)?;
        // Writer thread: stops when the channel closes (process dropped) or a write fails.
        let (wtx, wrx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            for chunk in wrx {
                if writer
                    .write_all(&chunk)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    return;
                }
            }
        });

        let this = Arc::new(Self {
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(wtx),
            killer: Mutex::new(killer),
            pid,
            job,
        });

        let (tx, rx) = mpsc::channel::<Msg>();

        // Reader: 16 KiB chunks until EOF (master dropped) or error.
        let tx_r = tx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx_r.send(Msg::Output(buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                }
            }
            let _ = tx_r.send(Msg::ReaderDone);
        });

        // Waiter: owns the child; on exit drops the master so the reader hits EOF.
        let weak = Arc::downgrade(&this);
        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code());
            if let Some(p) = weak.upgrade() {
                p.master.lock().unwrap_or_else(|e| e.into_inner()).take();
            }
            let _ = tx.send(Msg::Exit(code));
        });

        // Dispatcher: the only caller of the callback. `Exited` is delivered only once
        // both the exit code and `ReaderDone` have been seen; both travel on the same
        // channel as all `Output`, so nothing can follow it.
        std::thread::spawn(move || {
            let (mut reader_done, mut exit) = (false, None);
            for msg in rx {
                match msg {
                    Msg::Output(b) => on_event(PtyEvent::Output(b)),
                    Msg::ReaderDone => reader_done = true,
                    Msg::Exit(c) => exit = Some(c),
                }
                if reader_done {
                    if let Some(c) = exit {
                        on_event(PtyEvent::Exited(c));
                        return;
                    }
                }
            }
        });

        Ok(this)
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        w.send(bytes.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "pty writer stopped"))
    }

    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        let guard = self.master.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(m) => m
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(io_err),
            None => Err(io::Error::new(io::ErrorKind::BrokenPipe, "process exited")),
        }
    }

    pub fn kill(&self) -> io::Result<()> {
        // portable-pty 0.9's Windows `WinChildKiller::kill` inverts the TerminateProcess
        // result (success comes back as `Err(last_os_error)`, often code 0 or stale). Its
        // own `WinChild::kill` discards the result too, so do the same. Exit is observed
        // through `PtyEvent::Exited`. The job ends the whole tree (spec 4.1).
        if self.job.as_ref().is_some_and(|j| j.terminate().is_ok()) {
            return Ok(());
        }
        let _ = self.killer.lock().unwrap_or_else(|e| e.into_inner()).kill();
        Ok(())
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    type Slot = Arc<Mutex<Option<Arc<PtyProcess>>>>;

    fn start(args: &[&str]) -> (Arc<PtyProcess>, mpsc::Receiver<PtyEvent>) {
        start_program("cmd.exe", args)
    }

    /// PING.EXE processes whose arguments contain `marker`.
    fn pings_with(marker: &str) -> usize {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
        );
        sys.processes()
            .values()
            .filter(|p| p.name().eq_ignore_ascii_case("PING.EXE"))
            .filter(|p| p.cmd().iter().any(|a| a == marker))
            .count()
    }

    fn wait_until(secs: u64, cond: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        cond()
    }

    #[test]
    fn kill_ends_the_whole_process_tree() {
        // A grandchild with its own (hidden) console is not attached to our ConPTY, so
        // TerminateProcess of the child alone leaves it running (spec 4.1: "end the process tree").
        let marker = format!("{}", 600 + std::process::id() % 300);
        let script = format!(
            "Start-Process -WindowStyle Hidden -FilePath ping.exe -ArgumentList '-n','{marker}','127.0.0.1'; Start-Sleep 600"
        );
        let (p, rx) = start_program("powershell.exe", &["-NoProfile", "-Command", &script]);
        assert!(
            wait_until(15, || pings_with(&marker) == 1),
            "grandchild never started"
        );
        p.kill().unwrap();
        let (_, exit) = collect_until_exit(&rx, 10);
        assert!(exit.is_some(), "no Exited after kill");
        let gone = wait_until(5, || pings_with(&marker) == 0);
        assert!(gone, "grandchild survived kill()");
    }

    fn ping_pid(marker: &str) -> Option<u32> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
        );
        sys.processes()
            .iter()
            .find(|(_, p)| {
                p.name().eq_ignore_ascii_case("PING.EXE") && p.cmd().iter().any(|a| a == marker)
            })
            .map(|(pid, _)| pid.as_u32())
    }

    #[test]
    fn git_bash_grandchild_is_in_our_job() {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::JobObjects::IsProcessInJob;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        let marker = format!("{}", 1300 + std::process::id() % 290);
        let cmd = format!("ping -n {marker} 127.0.0.1");
        let bash = r"C:\Program Files\Git\bin\bash.exe";
        let (p, rx) = start_program(bash, &["-c", &cmd]);
        assert!(wait_until(15, || ping_pid(&marker).is_some()), "no ping");
        let pid = ping_pid(&marker).unwrap();
        let job = p.job.as_ref().expect("job").0;
        let mut inside = windows::core::BOOL(0);
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).unwrap();
            IsProcessInJob(h, Some(job), &mut inside).unwrap();
            let _ = CloseHandle(h);
        }
        p.kill().unwrap();
        let _ = collect_until_exit(&rx, 10);
        assert!(inside.as_bool(), "Git Bash's child broke away from the job");
    }

    #[test]
    fn kill_ends_git_bash_children_too() {
        // Hooks run through Git Bash (MSYS2). Its spawns must stay inside the job.
        let marker = format!("{}", 300 + std::process::id() % 290);
        let cmd = format!("ping -n {marker} 127.0.0.1; sleep 600");
        let bash = r"C:\Program Files\Git\bin\bash.exe";
        let (p, rx) = start_program(bash, &["-c", &cmd]);
        assert!(
            wait_until(15, || pings_with(&marker) == 1),
            "bash child never started"
        );
        p.kill().unwrap();
        let _ = collect_until_exit(&rx, 10);
        let gone = wait_until(5, || pings_with(&marker) == 0);
        assert!(gone, "Git Bash child survived kill()");
    }

    fn start_program(program: &str, args: &[&str]) -> (Arc<PtyProcess>, mpsc::Receiver<PtyEvent>) {
        let spec = SpawnSpec {
            program: PathBuf::from(program),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: vec![("FOO".into(), "bar".into())],
            env_remove: vec!["CLAUDE_CODE_CHILD_SESSION".into()],
            rows: 24,
            cols: 80,
            kill_tree: true,
        };
        let (tx, rx) = mpsc::channel();
        let slot: Slot = Arc::new(Mutex::new(None));
        let slot_cb = slot.clone();
        let mut answered = false;
        let mut pending = false;
        let p = PtyProcess::spawn(&spec, move |ev| {
            if let PtyEvent::Output(b) = &ev {
                if !answered && b.windows(4).any(|w| w == b"\x1b[6n") {
                    pending = true;
                }
            }
            if pending {
                if let Some(p) = slot_cb.lock().unwrap().as_ref() {
                    p.write(b"\x1b[1;1R").unwrap();
                    pending = false;
                    answered = true;
                }
            }
            let _ = tx.send(ev);
        })
        .unwrap();
        *slot.lock().unwrap() = Some(p.clone());
        // If the cursor query arrived before the slot was filled, answer now
        // (a second report is harmless).
        let _ = p.write(b"\x1b[1;1R");
        (p, rx)
    }

    fn collect_until_exit(
        rx: &mpsc::Receiver<PtyEvent>,
        secs: u64,
    ) -> (Vec<u8>, Option<Option<u32>>) {
        let deadline = Instant::now() + Duration::from_secs(secs);
        let mut out = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(PtyEvent::Output(b)) => out.extend(b),
                Ok(PtyEvent::Exited(c)) => return (out, Some(c)),
                Err(_) => break,
            }
        }
        (out, None)
    }

    #[test]
    fn echo_outputs_then_exits_zero_and_nothing_follows() {
        let (_p, rx) = start(&["/c", "echo hello"]);
        let (out, exit) = collect_until_exit(&rx, 10);
        assert!(String::from_utf8_lossy(&out).contains("hello"), "{out:?}");
        assert_eq!(exit, Some(Some(0)));
        assert!(
            rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "event after Exited"
        );
    }

    #[test]
    fn kill_leads_to_exited() {
        let (p, rx) = start(&["/k"]);
        std::thread::sleep(Duration::from_millis(500));
        p.kill().unwrap();
        let (_, exit) = collect_until_exit(&rx, 5);
        assert!(exit.is_some(), "no Exited after kill");
    }

    #[test]
    fn resize_live_process() {
        let (p, rx) = start(&["/k"]);
        std::thread::sleep(Duration::from_millis(500));
        p.resize(30, 100).unwrap();
        p.kill().unwrap();
        let (_, exit) = collect_until_exit(&rx, 5);
        assert!(exit.is_some());
    }

    #[test]
    fn large_write_returns_immediately() {
        let (p, rx) = start(&["/k"]);
        std::thread::sleep(Duration::from_millis(500));
        let big = vec![b'x'; 2 * 1024 * 1024];
        let t = Instant::now();
        p.write(&big).unwrap();
        assert!(t.elapsed() < Duration::from_millis(50), "{:?}", t.elapsed());
        p.kill().unwrap();
        let _ = collect_until_exit(&rx, 10);
    }

    #[test]
    fn without_kill_tree_there_is_no_job() {
        let mut spec = SpawnSpec {
            program: PathBuf::from("cmd.exe"),
            args: vec!["/k".into()],
            cwd: std::env::temp_dir(),
            env: vec![],
            env_remove: vec![],
            rows: 24,
            cols: 80,
            kill_tree: false,
        };
        let (tx, rx) = mpsc::channel();
        let p = PtyProcess::spawn(&spec, move |ev| {
            let _ = tx.send(ev);
        })
        .unwrap();
        assert!(p.job.is_none());
        let _ = p.write(b"[1;1R"); // answer ConPTY's cursor query
        p.kill().unwrap();
        let (_, exit) = collect_until_exit(&rx, 10);
        assert!(exit.is_some());
        spec.kill_tree = true;
        let (tx, _rx) = mpsc::channel();
        let p = PtyProcess::spawn(&spec, move |ev| {
            let _ = tx.send(ev);
        })
        .unwrap();
        assert!(p.job.is_some());
        p.kill().unwrap();
    }

    #[test]
    fn pid_is_some() {
        let (p, rx) = start(&["/k"]);
        assert!(p.pid().is_some());
        p.kill().unwrap();
        let _ = collect_until_exit(&rx, 5);
    }
}
