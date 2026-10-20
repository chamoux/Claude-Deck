//! Core, Input, Effect: all decisions of the helper (spec §4, §5, §8).
//!
//! `Core` is a synchronous state machine: `handle(input, now_ms)` returns the
//! effects the runtime must carry out. It never touches processes, pipes or
//! files (it only checks that a project folder exists).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use deck_core::claude_dir::PastSession;
use deck_core::hook::{parse_hook, HookEvent, StartSource};
use deck_core::state::SessionStatus;
use deck_core::store::{new_id, SavedSession, SavedState, ShellKind};
use deck_proto::messages::{
    encode_event, ClientKind, ConfirmAction, Event, GitResult, Request, SessionView, ShellView,
    StateView, Target,
};
use deck_proto::Frame;

use crate::dsr::{cursor_report, DsrFilter};
use crate::pty::SpawnSpec;
use crate::screen::Screen;

pub type ClientId = u64;

/// Program and arguments of the project shell for a shell setting.
pub type ShellLookup = fn(ShellKind) -> (PathBuf, Vec<String>);

const START_ROWS: u16 = 40;
const START_COLS: u16 = 120;
const ASK_MESSAGE: &str = "Claude may be working. Stop it anyway?";
const CANT_RESUME: &str = "Can't resume";
/// The confirm label of Claude's dialogs before it starts: "Yes, I trust this
/// folder" and "Yes, I trust these settings". Both default to "No, exit", so a
/// plain Enter quits Claude (spike 2, 2026-10-20).
const TRUST_PROMPT: &str = "Yes, I trust";
const NOT_TRUSTED: &str = "Claude quit: the folder was not trusted";
/// A change of `last_active_ms` alone is saved at most this often.
const TOUCH_SAVE_MS: i64 = 30_000;

#[derive(Debug)]
pub enum Input {
    WindowConnected(ClientId),
    ClientGone(ClientId),
    Request(ClientId, Request),
    TerminalInput {
        index: u32,
        bytes: Vec<u8>,
    },
    PtyOutput {
        target: Target,
        bytes: Vec<u8>,
    },
    PtyExited {
        target: Target,
    },
    SpawnFailed {
        target: Target,
        error: String,
    },
    /// The runtime sends one every 250 ms.
    Tick,
    GitDone {
        client: ClientId,
        project_id: String,
        result: GitResult,
    },
    PastDone {
        client: ClientId,
        project_id: String,
        sessions: Vec<PastSession>,
    },
    MemoryMeasured {
        awake: u32,
        bytes: u64,
    },
}

#[derive(Debug)]
pub enum Effect {
    Spawn {
        target: Target,
        spec: SpawnSpec,
    },
    Write {
        target: Target,
        bytes: Vec<u8>,
    },
    Resize {
        target: Target,
        rows: u16,
        cols: u16,
    },
    Kill {
        target: Target,
    },
    Send {
        client: ClientId,
        frame: Frame,
    },
    /// To every connected window.
    Broadcast {
        frame: Frame,
    },
    /// The runtime writes `HOOK_ACK` and closes.
    HookAck {
        client: ClientId,
    },
    RunGit {
        client: ClientId,
        project_id: String,
        path: PathBuf,
    },
    ListPast {
        client: ClientId,
        project_id: String,
        path: PathBuf,
        exclude: HashSet<String>,
    },
    /// Claude sessions only. The runtime maps targets to pids.
    MeasureMemory {
        targets: Vec<Target>,
    },
    /// The runtime debounces and writes on a blocking thread.
    Save {
        state: SavedState,
    },
    Exit,
}

#[derive(Debug, Clone)]
pub struct CoreConfig {
    pub claude_exe: PathBuf,
    pub hooks_file: PathBuf,
    /// The project shell, for every shell setting, unless `Core::with_shell_lookup`
    /// is used (as `main` does) to follow the setting.
    pub shell: (PathBuf, Vec<String>),
    /// Gap between the two Ctrl+C of a sleep (300).
    pub ctrl_c_gap_ms: i64,
    /// Time after which a sleeping process is killed (5000).
    pub sleep_grace_ms: i64,
    /// With no window and no awake session for this long, the shells are
    /// closed and the helper exits (10000).
    pub idle_exit_ms: i64,
    /// Period of memory measurements (5000).
    pub memory_every_ms: i64,
    /// The helper's pipe, passed to Claude sessions as CLAUDE_DECK_PIPE so their
    /// hooks reach this helper (tests run helpers on their own pipe names).
    pub pipe_name: Option<String>,
}

struct Sleep {
    since_ms: i64,
    second_sent: bool,
}

/// A terminal whose process is (or is about to be) running.
struct Live {
    seq: u64,
    index: Option<u32>,
    screen: Screen,
    dsr: DsrFilter,
    /// `Some` for Claude sessions, `None` for shells.
    status: Option<SessionStatus>,
    /// Launched with `--resume` (else `--session-id`).
    resumed: bool,
    /// This launch is the retry with the other flag (see `pty_exited`).
    retried: bool,
    /// Who started it: errors of a retry go there.
    client: ClientId,
    /// A SessionStart hook arrived.
    got_start: bool,
    /// Claude showed its trust dialog before starting.
    trust_seen: bool,
    sleep: Option<Sleep>,
    kill_sent: bool,
    /// A Wake arrived while it was stopping: start it again after the exit.
    wake_after_exit: Option<ClientId>,
}

pub struct Core {
    state: SavedState,
    config: CoreConfig,
    shell_lookup: Option<ShellLookup>,
    live: HashMap<Target, Live>,
    by_index: HashMap<u32, Target>,
    viewers: HashMap<Target, BTreeSet<ClientId>>,
    windows: BTreeSet<ClientId>,
    /// Per deck id: why the last wake failed.
    errors: HashMap<String, String>,
    next_index: u32,
    next_seq: u64,
    keep_running: Option<bool>,
    empty_since_ms: Option<i64>,
    last_memory_ms: Option<i64>,
    /// Since when the saved state has changes (last-active times) not yet saved.
    dirty_since_ms: Option<i64>,
    exit_sent: bool,
}

enum SpawnError {
    UnknownSession,
    UnknownProject,
    FolderMissing,
}

fn session_target(deck_id: &str) -> Target {
    Target::Session {
        deck_id: deck_id.to_string(),
    }
}

fn shell_target(project_id: &str) -> Target {
    Target::Shell {
        project_id: project_id.to_string(),
    }
}

fn send(out: &mut Vec<Effect>, client: ClientId, e: &Event) {
    out.push(Effect::Send {
        client,
        frame: encode_event(e),
    });
}

fn broadcast(out: &mut Vec<Effect>, e: &Event) {
    out.push(Effect::Broadcast {
        frame: encode_event(e),
    });
}

fn error_event(message: &str) -> Event {
    Event::Error {
        message: message.to_string(),
    }
}

fn shell_changed(project_id: &str, running: bool) -> Event {
    Event::ShellChanged {
        shell: ShellView {
            project_id: project_id.to_string(),
            running,
        },
    }
}

/// Asks the window to confirm stopping busy sessions (spec §4.2, §4.6, §5).
fn ask(out: &mut Vec<Effect>, client: ClientId, action: ConfirmAction) {
    let message = ASK_MESSAGE.to_string();
    send(out, client, &Event::NeedsConfirm { action, message });
}

/// True when `bytes` holds only what the window's terminal sends by itself:
/// focus in/out, mouse reports, and replies to queries (a CSI with a private
/// marker `< = > ?`, or an OSC/DCS/APC string). Claude turns on focus events and
/// mouse tracking, so these arrive without a key press and must not count as
/// the user answering (spec §4.2 "Keyboard input").
fn only_terminal_reports(mut bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    while let Some(rest) = bytes.strip_prefix(b"\x1b") {
        let next = match rest.split_first() {
            Some((b'[', csi)) => skip_csi_report(csi),
            Some((b']' | b'P' | b'_', body)) => skip_string(body),
            _ => None,
        };
        match next {
            Some(r) => bytes = r,
            None => return false,
        }
    }
    bytes.is_empty()
}

/// After `ESC [`: a focus event, an X10 mouse report, or a CSI with a private marker.
fn skip_csi_report(b: &[u8]) -> Option<&[u8]> {
    match b.first()? {
        b'I' | b'O' => Some(&b[1..]),
        b'M' => b.get(4..),
        0x3c..=0x3f => {
            let end = b.iter().position(|c| (0x40..=0x7e).contains(c))?;
            Some(&b[end + 1..])
        }
        _ => None,
    }
}

/// An OSC/DCS/APC body up to BEL or `ESC \`.
fn skip_string(b: &[u8]) -> Option<&[u8]> {
    let end = b.iter().enumerate().find_map(|(i, &c)| match c {
        0x07 => Some(i + 1),
        0x1b if b.get(i + 1) == Some(&b'\\') => Some(i + 2),
        _ => None,
    })?;
    Some(&b[end..])
}

/// Claude's trust dialog comes before any hook and quits on a plain Enter, so
/// it is shown as Needs you. True when it was just spotted.
fn spot_trust_prompt(live: &mut Live, now: i64) -> bool {
    let Some(status) = live.status.as_mut() else {
        return false;
    };
    if live.got_start || live.trust_seen || !live.screen.contents().contains(TRUST_PROMPT) {
        return false;
    }
    live.trust_seen = true;
    let message = TRUST_PROMPT.to_string();
    status.apply(&HookEvent::NeedsYou { message }, now);
    true
}

/// Claude quit before its SessionStart hook, and not because we stopped it
/// (a sleep or a kill says nothing about the conversation).
fn quit_before_start(live: &Live) -> bool {
    !live.got_start && live.sleep.is_none() && !live.kill_sent
}

impl Core {
    pub fn new(state: SavedState, config: CoreConfig) -> Self {
        Self {
            state,
            config,
            shell_lookup: None,
            live: HashMap::new(),
            by_index: HashMap::new(),
            viewers: HashMap::new(),
            windows: BTreeSet::new(),
            errors: HashMap::new(),
            next_index: 0,
            next_seq: 0,
            keep_running: None,
            empty_since_ms: None,
            last_memory_ms: None,
            dirty_since_ms: None,
            exit_sent: false,
        }
    }

    /// Makes the project shell follow the shell setting (`main` passes
    /// `locate::shell_command`). Without it `CoreConfig.shell` is always used.
    pub fn with_shell_lookup(mut self, lookup: ShellLookup) -> Self {
        self.shell_lookup = Some(lookup);
        self
    }

    pub fn state(&self) -> &SavedState {
        &self.state
    }

    pub fn handle(&mut self, input: Input, now: i64) -> Vec<Effect> {
        let mut out = Vec::new();
        let is_tick = matches!(input, Input::Tick);
        match input {
            Input::WindowConnected(c) => self.window_connected(c, &mut out),
            Input::ClientGone(c) => self.client_gone(c, &mut out),
            Input::Request(c, r) => self.request(c, r, now, &mut out),
            Input::TerminalInput { index, bytes } => {
                self.terminal_input(index, bytes, now, &mut out)
            }
            Input::PtyOutput { target, bytes } => self.pty_output(&target, &bytes, now, &mut out),
            Input::PtyExited { target } => self.pty_exited(&target, &mut out),
            Input::SpawnFailed { target, error } => self.spawn_failed(&target, &error, &mut out),
            Input::Tick => self.tick(now, &mut out),
            Input::GitDone {
                client,
                project_id,
                result,
            } => {
                send(&mut out, client, &Event::GitStatus { project_id, result });
            }
            Input::PastDone {
                client,
                project_id,
                sessions,
            } => {
                send(
                    &mut out,
                    client,
                    &Event::PastSessions {
                        project_id,
                        sessions,
                    },
                );
            }
            Input::MemoryMeasured { awake, bytes } => {
                broadcast(&mut out, &Event::Memory { awake, bytes });
            }
        }
        self.track_empty(now);
        if is_tick {
            self.check_idle_exit(now, &mut out);
        }
        out
    }

    // ---- views and saving --------------------------------------------------

    fn view_of(&self, saved: &SavedSession) -> SessionView {
        let status = self
            .live
            .get(&session_target(&saved.id))
            .and_then(|l| l.status)
            .unwrap_or_else(SessionStatus::asleep);
        SessionView {
            deck_id: saved.id.clone(),
            project_id: saved.project_id.clone(),
            name: saved.name.clone(),
            state: status.state,
            background_work: status.background_work,
            error: self.errors.get(&saved.id).cloned(),
        }
    }

    fn view(&self, deck_id: &str) -> Option<SessionView> {
        self.state.session(deck_id).map(|s| self.view_of(s))
    }

    fn state_view(&self) -> StateView {
        let mut shells: Vec<(u64, ShellView)> = self
            .live
            .iter()
            .filter_map(|(t, l)| match t {
                Target::Shell { project_id } => Some((
                    l.seq,
                    ShellView {
                        project_id: project_id.clone(),
                        running: true,
                    },
                )),
                Target::Session { .. } => None,
            })
            .collect();
        shells.sort_by_key(|(seq, _)| *seq);
        StateView {
            projects: self.state.projects.clone(),
            sessions: self
                .state
                .sessions
                .iter()
                .map(|s| self.view_of(s))
                .collect(),
            shells: shells.into_iter().map(|(_, s)| s).collect(),
            settings: self.state.settings.clone(),
            groups: self.state.groups.clone(),
            missing_folders: self
                .state
                .projects
                .iter()
                .filter(|p| !Path::new(&p.path).is_dir())
                .map(|p| p.id.clone())
                .collect(),
        }
    }

    fn send_state(&self, client: ClientId, out: &mut Vec<Effect>) {
        send(
            out,
            client,
            &Event::State {
                state: self.state_view(),
            },
        );
    }

    fn broadcast_state(&self, out: &mut Vec<Effect>) {
        broadcast(
            out,
            &Event::State {
                state: self.state_view(),
            },
        );
    }

    /// Every save goes through here; it also covers pending last-active times.
    fn save(&mut self, out: &mut Vec<Effect>) {
        self.dirty_since_ms = None;
        out.push(Effect::Save {
            state: self.state.clone(),
        });
    }

    /// Ok: save and broadcast the state. Err: tell only the requester.
    fn layout_result(
        &mut self,
        client: ClientId,
        result: Result<(), deck_core::store::LayoutError>,
        out: &mut Vec<Effect>,
    ) {
        match result {
            Ok(()) => self.save_and_state(out),
            Err(err) => send(out, client, &error_event(&err.to_string())),
        }
    }

    fn save_and_state(&mut self, out: &mut Vec<Effect>) {
        self.save(out);
        self.broadcast_state(out);
    }

    /// Updates the last-active time; a later `save` or the periodic one stores it.
    fn touch(&mut self, deck_id: &str, now: i64) {
        self.state.touch(deck_id, now);
        self.dirty_since_ms.get_or_insert(now);
    }

    fn broadcast_session(&self, deck_id: &str, out: &mut Vec<Effect>) {
        if let Some(session) = self.view(deck_id) {
            broadcast(out, &Event::SessionChanged { session });
        }
    }

    /// Live targets in the order they were started.
    fn live_targets(&self) -> Vec<Target> {
        let mut v: Vec<(u64, Target)> = self.live.iter().map(|(t, l)| (l.seq, t.clone())).collect();
        v.sort_by_key(|(seq, _)| *seq);
        v.into_iter().map(|(_, t)| t).collect()
    }

    // ---- connections -------------------------------------------------------

    fn window_connected(&mut self, client: ClientId, out: &mut Vec<Effect>) {
        self.windows.insert(client);
        self.keep_running = None;
        self.send_state(client, out);
    }

    fn client_gone(&mut self, client: ClientId, out: &mut Vec<Effect>) {
        for set in self.viewers.values_mut() {
            set.remove(&client);
        }
        if self.windows.remove(&client)
            && self.windows.is_empty()
            && self.keep_running != Some(true)
        {
            self.kill_all_shells(out);
        }
    }

    fn kill_all_shells(&mut self, out: &mut Vec<Effect>) {
        for t in self.live_targets() {
            if matches!(t, Target::Shell { .. }) {
                self.kill(&t, out);
            }
        }
    }

    fn kill(&mut self, target: &Target, out: &mut Vec<Effect>) {
        if let Some(l) = self.live.get_mut(target) {
            if !l.kill_sent {
                l.kill_sent = true;
                out.push(Effect::Kill {
                    target: target.clone(),
                });
            }
        }
    }

    // ---- requests ----------------------------------------------------------

    fn request(&mut self, client: ClientId, req: Request, now: i64, out: &mut Vec<Effect>) {
        match req {
            Request::Hello {
                client: ClientKind::Window,
            } => self.window_connected(client, out),
            Request::Hello {
                client: ClientKind::Hook,
            } => {}
            Request::GetState => self.send_state(client, out),
            Request::AddProject { path } => self.add_project(client, &path, out),
            Request::RemoveProject { project_id, force } => {
                self.remove_project(client, &project_id, force, now, out)
            }
            Request::NewSession { project_id } => self.new_session(client, &project_id, now, out),
            Request::Wake { deck_id } => self.wake(client, &deck_id, out),
            Request::Sleep { deck_id, force } => {
                self.sleep_request(client, &deck_id, force, now, out)
            }
            Request::CloseSession { deck_id, force } => {
                self.close_session(client, &deck_id, force, now, out)
            }
            Request::Rename { deck_id, name } => {
                if self.state.session(&deck_id).is_none() {
                    send(out, client, &error_event("Unknown session"));
                    return;
                }
                self.state.rename(&deck_id, &name);
                self.save(out);
                self.broadcast_session(&deck_id, out);
            }
            Request::Attach { target, rows, cols } => self.attach(client, target, rows, cols, out),
            Request::Detach { target } => {
                if let Some(set) = self.viewers.get_mut(&target) {
                    set.remove(&client);
                }
                send(out, client, &Event::Detached { target });
            }
            Request::Resize { target, rows, cols } => self.resize(&target, rows, cols, out),
            Request::CloseShell { project_id } => self.kill(&shell_target(&project_id), out),
            Request::SetShellPanel { project_id, panel } => {
                self.state.set_shell_panel(&project_id, panel);
                self.save_and_state(out);
            }
            Request::SetProjectLayout { project_id, layout } => {
                let r = self.state.set_project_layout(&project_id, layout);
                self.layout_result(client, r, out);
            }
            Request::AddGroup { name } => {
                self.state.add_group(&name);
                self.save_and_state(out);
            }
            Request::RenameGroup { group_id, name } => {
                let r = self.state.rename_group(&group_id, &name);
                self.layout_result(client, r, out);
            }
            Request::RemoveGroup { group_id } => {
                let r = self.state.remove_group(&group_id);
                self.layout_result(client, r, out);
            }
            Request::SetGroupLayout { group_id, layout } => {
                let r = self.state.set_group_layout(&group_id, layout);
                self.layout_result(client, r, out);
            }
            Request::GitStatus { project_id } => match self.state.project(&project_id) {
                Some(p) => out.push(Effect::RunGit {
                    client,
                    project_id,
                    path: PathBuf::from(&p.path),
                }),
                None => send(out, client, &error_event("Unknown project")),
            },
            Request::ListPastSessions { project_id } => match self.state.project(&project_id) {
                Some(p) => out.push(Effect::ListPast {
                    client,
                    project_id,
                    path: PathBuf::from(&p.path),
                    exclude: self.state.conversation_ids(),
                }),
                None => send(out, client, &error_event("Unknown project")),
            },
            Request::OpenPastSession {
                project_id,
                conversation_id,
            } => self.open_past(client, &project_id, &conversation_id, now, out),
            Request::SetSettings { settings } => {
                self.state.set_settings(settings);
                self.save_and_state(out);
            }
            Request::WindowClosing { keep_running } => self.window_closing(keep_running, now, out),
            Request::HookEvent { deck_id, payload } => {
                self.apply_hook(&deck_id, &payload, now, out);
                out.push(Effect::HookAck { client });
            }
        }
    }

    // ---- projects ----------------------------------------------------------

    fn add_project(&mut self, client: ClientId, path: &str, out: &mut Vec<Effect>) {
        if !Path::new(path).is_dir() {
            send(out, client, &error_event("Folder not found"));
            return;
        }
        self.state.add_project(path);
        self.save_and_state(out);
    }

    fn remove_project(
        &mut self,
        client: ClientId,
        project_id: &str,
        force: bool,
        now: i64,
        out: &mut Vec<Effect>,
    ) {
        if self.state.project(project_id).is_none() {
            send(out, client, &error_event("Unknown project"));
            return;
        }
        let ids: Vec<String> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.project_id == project_id)
            .map(|s| s.id.clone())
            .collect();
        if ids.iter().any(|id| self.needs_confirm(id, force)) {
            let project_id = project_id.to_string();
            ask(out, client, ConfirmAction::RemoveProject { project_id });
            return;
        }
        for id in &ids {
            self.begin_sleep(&session_target(id), now, out);
            self.errors.remove(id);
        }
        self.kill(&shell_target(project_id), out);
        self.state.remove_project(project_id);
        self.save_and_state(out);
    }

    /// The error, plus a State so the project shows "Folder missing" (spec §8).
    fn folder_missing(&self, client: ClientId, out: &mut Vec<Effect>) {
        send(out, client, &error_event("Folder missing"));
        self.broadcast_state(out);
    }

    // ---- starting ----------------------------------------------------------

    fn new_session(&mut self, client: ClientId, project_id: &str, now: i64, out: &mut Vec<Effect>) {
        let Some(project) = self.state.project(project_id) else {
            send(out, client, &error_event("Unknown project"));
            return;
        };
        if !Path::new(&project.path).is_dir() {
            self.folder_missing(client, out);
            return;
        }
        let id = new_id();
        self.state.add_session(project_id, id.clone(), now);
        self.save(out);
        let size = (START_ROWS, START_COLS);
        self.spawn(&session_target(&id), false, size, client, out);
    }

    fn open_past(
        &mut self,
        client: ClientId,
        project_id: &str,
        conversation_id: &str,
        now: i64,
        out: &mut Vec<Effect>,
    ) {
        let Some(project) = self.state.project(project_id) else {
            send(out, client, &error_event("Unknown project"));
            return;
        };
        if !Path::new(&project.path).is_dir() {
            self.folder_missing(client, out);
            return;
        }
        let deck_id = self
            .state
            .add_past_session(project_id, conversation_id, now)
            .id
            .clone();
        self.save(out);
        self.wake(client, &deck_id, out);
    }

    /// Starts an asleep session. A session that is stopping starts again once
    /// it has exited; an awake one is left alone (one Claude per deck id).
    fn wake(&mut self, client: ClientId, deck_id: &str, out: &mut Vec<Effect>) {
        let Some(saved) = self.state.session(deck_id) else {
            send(out, client, &error_event("Unknown session"));
            return;
        };
        // Never --resume a conversation Claude has not saved yet.
        let resume = saved.has_conversation;
        let target = session_target(deck_id);
        if let Some(live) = self.live.get_mut(&target) {
            if live.sleep.is_some() || live.kill_sent {
                live.wake_after_exit = Some(client);
            }
            return;
        }
        self.spawn(&target, resume, (START_ROWS, START_COLS), client, out);
    }

    /// Starts the process of `target` unless it is live. Returns true when it is live afterwards.
    fn spawn(
        &mut self,
        target: &Target,
        resume: bool,
        size: (u16, u16),
        client: ClientId,
        out: &mut Vec<Effect>,
    ) -> bool {
        if self.live.contains_key(target) {
            return true;
        }
        let spec = match self.spawn_spec(target, resume, size) {
            Ok(spec) => spec,
            Err(SpawnError::UnknownSession) => {
                send(out, client, &error_event("Unknown session"));
                return false;
            }
            Err(SpawnError::UnknownProject) => {
                send(out, client, &error_event("Unknown project"));
                return false;
            }
            Err(SpawnError::FolderMissing) => {
                self.folder_missing(client, out);
                return false;
            }
        };
        self.insert_live(target, resume, size, client);
        out.push(Effect::Spawn {
            target: target.clone(),
            spec,
        });
        self.announce_started(target, out);
        true
    }

    fn spawn_spec(
        &self,
        target: &Target,
        resume: bool,
        (rows, cols): (u16, u16),
    ) -> Result<SpawnSpec, SpawnError> {
        let project_id = match target {
            Target::Session { deck_id } => {
                let saved = self
                    .state
                    .session(deck_id)
                    .ok_or(SpawnError::UnknownSession)?;
                &saved.project_id
            }
            Target::Shell { project_id } => project_id,
        };
        let project = self
            .state
            .project(project_id)
            .ok_or(SpawnError::UnknownProject)?;
        let cwd = PathBuf::from(&project.path);
        if !cwd.is_dir() {
            return Err(SpawnError::FolderMissing);
        }
        let mut env = vec![
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
        ];
        let (program, args) = match target {
            Target::Session { deck_id } => self.claude_command(deck_id, resume, &mut env),
            Target::Shell { .. } => self.shell_command(),
        };
        Ok(SpawnSpec {
            program,
            args,
            cwd,
            env,
            env_remove: vec![
                "CLAUDE_CODE_CHILD_SESSION".to_string(),
                "CLAUDECODE".to_string(),
            ],
            rows,
            cols,
            // Sessions end with their whole tree; shells must not take GUI apps down.
            kill_tree: matches!(target, Target::Session { .. }),
        })
    }

    /// `--resume <conversation>` or `--session-id <conversation>`, plus the
    /// session's environment.
    fn claude_command(
        &self,
        deck_id: &str,
        resume: bool,
        env: &mut Vec<(String, String)>,
    ) -> (PathBuf, Vec<String>) {
        let claude = match self.state.settings.claude_path.as_deref() {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => self.config.claude_exe.clone(),
        };
        let conversation = self
            .state
            .session(deck_id)
            .map_or_else(|| deck_id.to_string(), |s| s.conversation_id.clone());
        let flag = if resume { "--resume" } else { "--session-id" };
        env.push(("CLAUDE_DECK_SESSION".to_string(), deck_id.to_string()));
        if let Some(pipe) = &self.config.pipe_name {
            env.push(("CLAUDE_DECK_PIPE".to_string(), pipe.clone()));
        }
        let args = vec![
            flag.to_string(),
            conversation,
            "--settings".to_string(),
            self.config.hooks_file.to_string_lossy().into_owned(),
        ];
        (claude, args)
    }

    fn shell_command(&self) -> (PathBuf, Vec<String>) {
        match self.shell_lookup {
            Some(lookup) => lookup(self.state.settings.shell),
            None => self.config.shell.clone(),
        }
    }

    fn insert_live(
        &mut self,
        target: &Target,
        resume: bool,
        (rows, cols): (u16, u16),
        client: ClientId,
    ) {
        let is_session = matches!(target, Target::Session { .. });
        self.next_seq += 1;
        self.live.insert(
            target.clone(),
            Live {
                seq: self.next_seq,
                index: None,
                screen: Screen::new(rows, cols, if is_session { 5000 } else { 1000 }),
                dsr: DsrFilter::new(),
                status: is_session.then(SessionStatus::started),
                resumed: resume && is_session,
                retried: false,
                client,
                got_start: false,
                trust_seen: false,
                sleep: None,
                kill_sent: false,
                wake_after_exit: None,
            },
        );
    }

    fn announce_started(&mut self, target: &Target, out: &mut Vec<Effect>) {
        match target {
            Target::Session { deck_id } => {
                self.errors.remove(deck_id);
                self.broadcast_session(deck_id, out);
            }
            Target::Shell { project_id } => broadcast(out, &shell_changed(project_id, true)),
        }
    }

    // ---- ending ------------------------------------------------------------

    fn remove_live(&mut self, target: &Target) -> Option<Live> {
        let live = self.live.remove(target)?;
        if let Some(i) = live.index {
            self.by_index.remove(&i);
        }
        self.viewers.remove(target);
        Some(live)
    }

    fn announce_gone(&self, target: &Target, out: &mut Vec<Effect>) {
        match target {
            Target::Session { deck_id } => self.broadcast_session(deck_id, out),
            Target::Shell { project_id } => broadcast(out, &shell_changed(project_id, false)),
        }
    }

    fn spawn_failed(&mut self, target: &Target, error: &str, out: &mut Vec<Effect>) {
        if self.remove_live(target).is_none() {
            return;
        }
        if let Target::Session { deck_id } = target {
            self.errors.insert(deck_id.clone(), error.to_string());
        }
        broadcast(out, &error_event(error));
        self.announce_gone(target, out);
    }

    fn pty_exited(&mut self, target: &Target, out: &mut Vec<Effect>) {
        let Some(live) = self.remove_live(target) else {
            return;
        };
        let Target::Session { deck_id } = target else {
            self.announce_gone(target, out);
            return;
        };
        let saved = self.state.session(deck_id).is_some();
        if saved && quit_before_start(&live) {
            if live.trust_seen {
                // "No, exit" on the trust dialog: the conversation is fine.
                self.errors.insert(deck_id.clone(), NOT_TRUSTED.to_string());
            } else if !live.retried {
                // Safety net (spike 3): `--resume` of a conversation with no
                // transcript, or `--session-id` of one that has one, quits at
                // once. Try the other flag once, for the same conversation.
                let size = (START_ROWS, START_COLS);
                if self.spawn(target, !live.resumed, size, live.client, out) {
                    if let Some(l) = self.live.get_mut(target) {
                        l.retried = true;
                    }
                    return;
                }
            } else {
                self.errors.insert(deck_id.clone(), CANT_RESUME.to_string());
            }
        }
        self.announce_gone(target, out);
        if let (true, Some(client)) = (saved, live.wake_after_exit) {
            self.wake(client, deck_id, out);
        }
    }

    // ---- hooks -------------------------------------------------------------

    fn apply_hook(&mut self, deck_id: &str, payload: &str, now: i64, out: &mut Vec<Effect>) {
        let Ok(call) = parse_hook(payload) else {
            return;
        };
        let target = session_target(deck_id);
        let Some(saved_before) = self.state.session(deck_id).cloned() else {
            return;
        };
        let view_before = self.view_of(&saved_before);
        let Some(live) = self.live.get_mut(&target) else {
            return;
        };
        if let Some(status) = live.status.as_mut() {
            status.apply(&call.event, now);
        }
        // Whether Claude has written a transcript to `--resume` (spike 3):
        // a fresh start has none until its first turn; /clear writes one at once.
        match &call.event {
            HookEvent::SessionStart { source } => {
                live.got_start = true;
                self.errors.remove(deck_id);
                self.state.set_conversation(deck_id, &call.conversation_id);
                match source {
                    StartSource::Startup => self.state.set_has_conversation(deck_id, false),
                    StartSource::Resume | StartSource::Clear => {
                        self.state.set_has_conversation(deck_id, true)
                    }
                    StartSource::Compact | StartSource::Other(_) => {}
                }
            }
            HookEvent::UserPromptSubmit { prompt } => {
                self.state.name_from_prompt(deck_id, prompt);
                self.state.set_has_conversation(deck_id, true);
            }
            HookEvent::ToolUse | HookEvent::Stop { .. } => {
                self.state.set_has_conversation(deck_id, true)
            }
            _ => {}
        }
        let changed = self.state.session(deck_id) != Some(&saved_before);
        self.touch(deck_id, now);
        if changed {
            self.save(out);
        }
        if let Some(after) = self.view(deck_id) {
            if after != view_before {
                broadcast(out, &Event::SessionChanged { session: after });
            }
        }
    }

    // ---- terminal traffic --------------------------------------------------

    fn pty_output(&mut self, target: &Target, bytes: &[u8], now: i64, out: &mut Vec<Effect>) {
        let Some(live) = self.live.get_mut(target) else {
            return;
        };
        let (clean, queries) = live.dsr.feed_at(bytes);
        // Each query is answered with the cursor where it was asked, before the
        // bytes after it are processed.
        let mut done = 0;
        for at in queries {
            live.screen.process(&clean[done..at]);
            done = at;
            let (row, col) = live.screen.cursor();
            out.push(Effect::Write {
                target: target.clone(),
                bytes: cursor_report(row, col),
            });
        }
        live.screen.process(&clean[done..]);
        let trust_now = spot_trust_prompt(live, now);
        if let (Some(index), false) = (live.index, clean.is_empty()) {
            for client in self.viewers.get(target).into_iter().flatten() {
                out.push(Effect::Send {
                    client: *client,
                    frame: Frame::Output {
                        session: index,
                        data: clean.clone(),
                    },
                });
            }
        }
        if let (true, Target::Session { deck_id }) = (trust_now, target) {
            self.broadcast_session(deck_id, out);
        }
    }

    fn terminal_input(&mut self, index: u32, bytes: Vec<u8>, now: i64, out: &mut Vec<Effect>) {
        let Some(target) = self.by_index.get(&index).cloned() else {
            return;
        };
        let Some(live) = self.live.get_mut(&target) else {
            return;
        };
        let keys = !only_terminal_reports(&bytes);
        out.push(Effect::Write {
            target: target.clone(),
            bytes,
        });
        if !keys {
            return;
        }
        if let (Some(status), Target::Session { deck_id }) = (live.status.as_mut(), &target) {
            let before = status.state;
            status.on_input(now);
            if status.state != before {
                self.broadcast_session(deck_id, out);
            }
        }
    }

    fn attach(
        &mut self,
        client: ClientId,
        target: Target,
        rows: u16,
        cols: u16,
        out: &mut Vec<Effect>,
    ) {
        if !self.live.contains_key(&target) {
            if let Target::Session { .. } = target {
                send(out, client, &error_event("Session is asleep"));
                return;
            }
            // A shell starts at the window's size.
            let size = match (rows, cols) {
                (0, _) | (_, 0) => (START_ROWS, START_COLS),
                size => size,
            };
            if !self.spawn(&target, false, size, client, out) {
                return;
            }
        }
        let live = self.live.get_mut(&target).expect("live after spawn");
        let index = match live.index {
            Some(i) => i,
            None => {
                let i = self.next_index;
                self.next_index += 1;
                live.index = Some(i);
                self.by_index.insert(i, target.clone());
                i
            }
        };
        self.viewers
            .entry(target.clone())
            .or_default()
            .insert(client);
        // Resize first so the snapshot has the window's size.
        self.resize(&target, rows, cols, out);
        let live = self.live.get_mut(&target).expect("still live");
        let snapshot = live.screen.snapshot();
        send(out, client, &Event::Attached { target, index });
        out.push(Effect::Send {
            client,
            frame: Frame::Output {
                session: index,
                data: snapshot,
            },
        });
    }

    fn resize(&mut self, target: &Target, rows: u16, cols: u16, out: &mut Vec<Effect>) {
        if rows == 0 || cols == 0 {
            return;
        }
        if let Some(live) = self.live.get_mut(target) {
            live.screen.resize(rows, cols);
            out.push(Effect::Resize {
                target: target.clone(),
                rows,
                cols,
            });
        }
    }

    // ---- sleeping ----------------------------------------------------------

    /// Stopping this session needs the user's yes: it is awake, busy (Working,
    /// Needs you or background work) and not already going to sleep.
    fn needs_confirm(&self, deck_id: &str, force: bool) -> bool {
        !force
            && self.live.get(&session_target(deck_id)).is_some_and(|l| {
                l.sleep.is_none() && l.status.is_some_and(|s| !s.can_sleep_without_asking())
            })
    }

    /// Writes the first Ctrl+C. Does nothing for shells or when already sleeping.
    fn begin_sleep(&mut self, target: &Target, now: i64, out: &mut Vec<Effect>) {
        if let Some(l) = self.live.get_mut(target) {
            if l.status.is_some() && l.sleep.is_none() {
                l.sleep = Some(Sleep {
                    since_ms: now,
                    second_sent: false,
                });
                out.push(Effect::Write {
                    target: target.clone(),
                    bytes: vec![3],
                });
            }
        }
    }

    fn sleep_request(
        &mut self,
        client: ClientId,
        deck_id: &str,
        force: bool,
        now: i64,
        out: &mut Vec<Effect>,
    ) {
        if self.state.session(deck_id).is_none() {
            send(out, client, &error_event("Unknown session"));
            return;
        }
        if self.needs_confirm(deck_id, force) {
            let deck_id = deck_id.to_string();
            ask(out, client, ConfirmAction::Sleep { deck_id });
            return;
        }
        self.begin_sleep(&session_target(deck_id), now, out);
    }

    fn close_session(
        &mut self,
        client: ClientId,
        deck_id: &str,
        force: bool,
        now: i64,
        out: &mut Vec<Effect>,
    ) {
        if self.state.session(deck_id).is_none() {
            send(out, client, &error_event("Unknown session"));
            return;
        }
        if self.needs_confirm(deck_id, force) {
            let deck_id = deck_id.to_string();
            ask(out, client, ConfirmAction::CloseSession { deck_id });
            return;
        }
        self.begin_sleep(&session_target(deck_id), now, out);
        self.state.remove_session(deck_id);
        self.errors.remove(deck_id);
        self.save_and_state(out);
    }

    fn window_closing(&mut self, keep_running: bool, now: i64, out: &mut Vec<Effect>) {
        self.keep_running = Some(keep_running);
        for target in self.live_targets() {
            match &target {
                Target::Session { deck_id } => {
                    if !keep_running || !self.needs_confirm(deck_id, false) {
                        self.begin_sleep(&target, now, out);
                    }
                }
                Target::Shell { .. } => {
                    if !keep_running {
                        self.kill(&target, out);
                    }
                }
            }
        }
    }

    // ---- time --------------------------------------------------------------

    fn tick(&mut self, now: i64, out: &mut Vec<Effect>) {
        let minutes = self.state.settings.auto_sleep_minutes;
        for target in self.live_targets() {
            let due = self.live.get(&target).is_some_and(|l| {
                l.sleep.is_none() && l.status.is_some_and(|s| s.should_auto_sleep(now, minutes))
            });
            if due {
                self.begin_sleep(&target, now, out);
            }
        }
        self.advance_sleeps(now, out);
        self.measure_memory(now, out);
        if self
            .dirty_since_ms
            .is_some_and(|t| now - t >= TOUCH_SAVE_MS)
        {
            self.save(out);
        }
    }

    fn advance_sleeps(&mut self, now: i64, out: &mut Vec<Effect>) {
        let (gap, grace) = (self.config.ctrl_c_gap_ms, self.config.sleep_grace_ms);
        for target in self.live_targets() {
            let Some(live) = self.live.get_mut(&target) else {
                continue;
            };
            let Some(sleep) = live.sleep.as_mut() else {
                continue;
            };
            if !sleep.second_sent && now - sleep.since_ms >= gap {
                sleep.second_sent = true;
                out.push(Effect::Write {
                    target: target.clone(),
                    bytes: vec![3],
                });
            }
            if now - sleep.since_ms >= grace {
                self.kill(&target, out);
            }
        }
    }

    /// While a window is connected: the awake Claude sessions (the footer, spec §3.3).
    fn measure_memory(&mut self, now: i64, out: &mut Vec<Effect>) {
        let due = self
            .last_memory_ms
            .is_none_or(|t| now - t >= self.config.memory_every_ms);
        if self.windows.is_empty() || !due {
            return;
        }
        self.last_memory_ms = Some(now);
        let targets = self
            .live_targets()
            .into_iter()
            .filter(|t| matches!(t, Target::Session { .. }))
            .collect();
        out.push(Effect::MeasureMemory { targets });
    }

    /// Shells do not keep the helper alive (spec §3.2, §5).
    fn track_empty(&mut self, now: i64) {
        let awake = self
            .live
            .keys()
            .any(|t| matches!(t, Target::Session { .. }));
        if self.windows.is_empty() && !awake {
            self.empty_since_ms.get_or_insert(now);
        } else {
            self.empty_since_ms = None;
        }
    }

    /// No window and no awake session for `idle_exit_ms`: the shells are closed
    /// (they live "until the helper exits", spec §5), and once nothing is live
    /// the helper saves what is pending and exits.
    fn check_idle_exit(&mut self, now: i64, out: &mut Vec<Effect>) {
        let Some(since) = self.empty_since_ms else {
            return;
        };
        if self.exit_sent || now - since < self.config.idle_exit_ms {
            return;
        }
        self.kill_all_shells(out);
        if self.live.is_empty() {
            if self.dirty_since_ms.is_some() {
                self.save(out);
            }
            self.exit_sent = true;
            out.push(Effect::Exit);
        }
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
