//! Messages between the window and the helper (spec §3.4).
//! Every message travels as JSON inside `Frame::Control`. Terminal bytes travel
//! separately as `Frame::Output` / `Frame::Input`, keyed by the index from `Event::Attached`.

use crate::Frame;
use deck_core::{
    claude_dir::PastSession,
    git::ChangeList,
    state::SessionState,
    store::{Group, Project, Settings, ShellPanel},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Window,
    Hook,
}

/// A terminal the window can attach: a Claude session (by deck id) or a project shell.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Session { deck_id: String },
    Shell { project_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello {
        client: ClientKind,
    },
    GetState,
    AddProject {
        path: String,
    },
    /// `force` = the user confirmed stopping the project's busy sessions.
    RemoveProject {
        project_id: String,
        #[serde(default)]
        force: bool,
    },
    NewSession {
        project_id: String,
    },
    Wake {
        deck_id: String,
    },
    /// `force` = the user confirmed sleeping a non-Idle session.
    Sleep {
        deck_id: String,
        force: bool,
    },
    CloseSession {
        deck_id: String,
        force: bool,
    },
    Rename {
        deck_id: String,
        name: String,
    },
    /// Starts the shell if needed; for a session the session must be awake.
    Attach {
        target: Target,
        rows: u16,
        cols: u16,
    },
    Detach {
        target: Target,
    },
    Resize {
        target: Target,
        rows: u16,
        cols: u16,
    },
    CloseShell {
        project_id: String,
    },
    SetShellPanel {
        project_id: String,
        panel: ShellPanel,
    },
    /// `None` clears the project's saved layout.
    SetProjectLayout {
        project_id: String,
        layout: Option<Value>,
    },
    AddGroup {
        name: String,
    },
    RenameGroup {
        group_id: String,
        name: String,
    },
    RemoveGroup {
        group_id: String,
    },
    SetGroupLayout {
        group_id: String,
        layout: Value,
    },
    GitStatus {
        project_id: String,
    },
    ListPastSessions {
        project_id: String,
    },
    OpenPastSession {
        project_id: String,
        conversation_id: String,
    },
    SetSettings {
        settings: Settings,
    },
    /// Idle sessions always sleep; `keep_running == false` also sleeps busy sessions and closes shells (spec §4.4, §5).
    WindowClosing {
        keep_running: bool,
    },
    /// From `deck-host hook`: Claude's raw hook JSON for the session with this deck id.
    HookEvent {
        deck_id: String,
        payload: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionView {
    pub deck_id: String,
    pub project_id: String,
    pub name: Option<String>,
    pub state: SessionState,
    pub background_work: bool,
    /// Set when the last wake failed, e.g. "Can't resume" (spec §8).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShellView {
    pub project_id: String,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateView {
    pub projects: Vec<Project>,
    pub sessions: Vec<SessionView>,
    pub shells: Vec<ShellView>,
    pub settings: Settings,
    /// Projects whose folder no longer exists (spec §8).
    pub missing_folders: Vec<String>,
    #[serde(default)]
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GitResult {
    Ok { list: ChangeList },
    NotGit,
    GitMissing,
    Error { message: String },
}

/// A request the helper refused because it would stop a busy session. Its JSON
/// plus `"force": true` is the request to send once the user confirms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfirmAction {
    Sleep { deck_id: String },
    CloseSession { deck_id: String },
    RemoveProject { project_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    State {
        state: StateView,
    },
    SessionChanged {
        session: SessionView,
    },
    ShellChanged {
        shell: ShellView,
    },
    /// The next Frame::Output for `index` is a full redraw (snapshot); reset the terminal first.
    Attached {
        target: Target,
        index: u32,
    },
    Detached {
        target: Target,
    },
    Memory {
        awake: u32,
        bytes: u64,
    },
    GitStatus {
        project_id: String,
        result: GitResult,
    },
    PastSessions {
        project_id: String,
        sessions: Vec<PastSession>,
    },
    /// Ask the user, e.g. "Claude may be working. Stop it anyway?"; on yes,
    /// resend `action` with `force: true` (spec §4.2, §4.6, §5).
    NeedsConfirm {
        action: ConfirmAction,
        message: String,
    },
    Error {
        message: String,
    },
}

pub fn encode_request(r: &Request) -> Frame {
    Frame::Control(serde_json::to_vec(r).expect("requests always serialise"))
}

pub fn encode_event(e: &Event) -> Frame {
    Frame::Control(serde_json::to_vec(e).expect("events always serialise"))
}

pub fn decode_request(json: &[u8]) -> Result<Request, serde_json::Error> {
    serde_json::from_slice(json)
}

pub fn decode_event(json: &[u8]) -> Result<Event, serde_json::Error> {
    serde_json::from_slice(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_core::git::{Change, ChangeKind};

    fn json_of<T: Serialize>(v: &T) -> String {
        serde_json::to_string(v).unwrap()
    }

    fn shell() -> Target {
        Target::Shell {
            project_id: "p".into(),
        }
    }

    fn panel() -> ShellPanel {
        ShellPanel {
            open: true,
            width: 300,
        }
    }

    #[test]
    fn wire_format_is_pinned() {
        let r = Request::Attach {
            target: shell(),
            rows: 40,
            cols: 120,
        };
        assert_eq!(
            json_of(&r),
            r#"{"type":"attach","target":{"kind":"shell","project_id":"p"},"rows":40,"cols":120}"#
        );
        assert_eq!(
            json_of(&Event::Memory { awake: 2, bytes: 5 }),
            r#"{"type":"memory","awake":2,"bytes":5}"#
        );
        assert_eq!(json_of(&GitResult::NotGit), r#"{"status":"not_git"}"#);
        assert_eq!(json_of(&Request::GetState), r#"{"type":"get_state"}"#);
    }

    #[test]
    fn every_request_round_trips() {
        let s = || "x".to_string();
        let all = vec![
            Request::Hello {
                client: ClientKind::Window,
            },
            Request::Hello {
                client: ClientKind::Hook,
            },
            Request::GetState,
            Request::AddProject { path: s() },
            Request::RemoveProject {
                project_id: s(),
                force: true,
            },
            Request::NewSession { project_id: s() },
            Request::Wake { deck_id: s() },
            Request::Sleep {
                deck_id: s(),
                force: true,
            },
            Request::CloseSession {
                deck_id: s(),
                force: false,
            },
            Request::Rename {
                deck_id: s(),
                name: s(),
            },
            Request::Attach {
                target: Target::Session { deck_id: s() },
                rows: 1,
                cols: 2,
            },
            Request::Detach { target: shell() },
            Request::Resize {
                target: shell(),
                rows: 3,
                cols: 4,
            },
            Request::CloseShell { project_id: s() },
            Request::SetShellPanel {
                project_id: s(),
                panel: panel(),
            },
            Request::SetProjectLayout {
                project_id: s(),
                layout: Some(serde_json::json!({"type":"pane"})),
            },
            Request::SetProjectLayout {
                project_id: s(),
                layout: None,
            },
            Request::AddGroup { name: s() },
            Request::RenameGroup {
                group_id: s(),
                name: s(),
            },
            Request::RemoveGroup { group_id: s() },
            Request::SetGroupLayout {
                group_id: s(),
                layout: serde_json::json!({"type":"pane"}),
            },
            Request::GitStatus { project_id: s() },
            Request::ListPastSessions { project_id: s() },
            Request::OpenPastSession {
                project_id: s(),
                conversation_id: s(),
            },
            Request::SetSettings {
                settings: Settings::default(),
            },
            Request::WindowClosing { keep_running: true },
            Request::HookEvent {
                deck_id: s(),
                payload: "{}".into(),
            },
        ];
        for r in all {
            let Frame::Control(bytes) = encode_request(&r) else {
                panic!("not control")
            };
            assert_eq!(decode_request(&bytes).unwrap(), r);
        }
    }

    #[test]
    fn layout_and_group_requests_are_pinned() {
        assert_eq!(
            json_of(&Request::AddGroup {
                name: "Billing".into()
            }),
            r#"{"type":"add_group","name":"Billing"}"#
        );
        let r = Request::SetGroupLayout {
            group_id: "g".into(),
            layout: serde_json::json!({"type":"pane","id":"p1","tabs":[],"active":0}),
        };
        let want = r#"{"type":"set_group_layout","group_id":"g","layout":{"type":"pane","id":"p1","tabs":[],"active":0}}"#;
        assert_eq!(json_of(&r), want);
        assert_eq!(decode_request(want.as_bytes()).unwrap(), r);
        let r = decode_request(br#"{"type":"set_project_layout","project_id":"p","layout":null}"#)
            .unwrap();
        assert_eq!(
            r,
            Request::SetProjectLayout {
                project_id: "p".into(),
                layout: None
            }
        );
    }

    #[test]
    fn state_with_groups_is_pinned_and_groups_default_to_empty() {
        let want = r#"{"type":"state","state":{"projects":[],"sessions":[],"shells":[],"settings":{"auto_sleep_minutes":30,"claude_path":null,"window":null,"shell":"power_shell"},"missing_folders":[],"groups":[{"id":"g1","name":"Billing","layout":{}}]}}"#;
        let Event::State { state } = decode_event(want.as_bytes()).unwrap() else {
            panic!()
        };
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.groups[0].name, "Billing");
        let e = Event::State { state };
        assert_eq!(json_of(&e), want);
        let old = want.replace(
            r#","groups":[{"id":"g1","name":"Billing","layout":{}}]"#,
            "",
        );
        let Event::State { state } = decode_event(old.as_bytes()).unwrap() else {
            panic!()
        };
        assert!(state.groups.is_empty());
    }

    #[test]
    fn remove_project_force_is_pinned_and_defaults_to_false() {
        let r = Request::RemoveProject {
            project_id: "p".into(),
            force: true,
        };
        assert_eq!(
            json_of(&r),
            r#"{"type":"remove_project","project_id":"p","force":true}"#
        );
        let old = decode_request(br#"{"type":"remove_project","project_id":"p"}"#).unwrap();
        assert_eq!(
            old,
            Request::RemoveProject {
                project_id: "p".into(),
                force: false
            }
        );
    }

    #[test]
    fn needs_confirm_wire_format_is_pinned() {
        let msg = "Claude may be working. Stop it anyway?";
        let cases = [
            (
                ConfirmAction::Sleep {
                    deck_id: "d".into(),
                },
                r#"{"type":"needs_confirm","action":{"type":"sleep","deck_id":"d"},"message":"Claude may be working. Stop it anyway?"}"#,
            ),
            (
                ConfirmAction::CloseSession {
                    deck_id: "d".into(),
                },
                r#"{"type":"needs_confirm","action":{"type":"close_session","deck_id":"d"},"message":"Claude may be working. Stop it anyway?"}"#,
            ),
            (
                ConfirmAction::RemoveProject {
                    project_id: "p".into(),
                },
                r#"{"type":"needs_confirm","action":{"type":"remove_project","project_id":"p"},"message":"Claude may be working. Stop it anyway?"}"#,
            ),
        ];
        for (action, want) in cases {
            let e = Event::NeedsConfirm {
                action,
                message: msg.into(),
            };
            assert_eq!(json_of(&e), want);
            assert_eq!(decode_event(want.as_bytes()).unwrap(), e);
        }
    }

    #[test]
    fn state_event_round_trips() {
        let state = StateView {
            projects: vec![Project {
                id: "p1".into(),
                path: "C:/code/a".into(),
                name: "a".into(),
                shell_panel: ShellPanel::default(),
                layout: Some(serde_json::json!({"type":"pane"})),
            }],
            sessions: vec![SessionView {
                deck_id: "d".into(),
                project_id: "p1".into(),
                name: Some("n".into()),
                state: SessionState::Idle,
                background_work: false,
                error: None,
            }],
            shells: vec![ShellView {
                project_id: "p1".into(),
                running: true,
            }],
            settings: Settings::default(),
            missing_folders: vec!["p1".into()],
            groups: vec![Group {
                id: "g1".into(),
                name: "Billing".into(),
                layout: serde_json::json!({}),
            }],
        };
        let e = Event::State { state };
        let Frame::Control(b) = encode_event(&e) else {
            panic!()
        };
        assert_eq!(decode_event(&b).unwrap(), e);
    }

    #[test]
    fn git_event_round_trips() {
        let list = ChangeList {
            items: vec![Change {
                kind: ChangeKind::Renamed,
                path: "b".into(),
                old_path: Some("a".into()),
                conflict: false,
            }],
            total: 1,
        };
        let e = Event::GitStatus {
            project_id: "p".into(),
            result: GitResult::Ok { list },
        };
        let Frame::Control(b) = encode_event(&e) else {
            panic!()
        };
        assert_eq!(decode_event(&b).unwrap(), e);
    }
}
