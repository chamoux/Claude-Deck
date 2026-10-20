//! The app's saved state, %LOCALAPPDATA%\ClaudeDeck\state.json (spec §7).

use crate::claude_dir::same_path;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const STATE_VERSION: u32 = 1;
pub const MIN_AUTO_SLEEP_MINUTES: u32 = 5;
pub const MAX_AUTO_SLEEP_MINUTES: u32 = 240;
const NAME_CHARS: usize = 40;
const MIN_SHELL_WIDTH: u32 = 240;
const MAX_SHELL_WIDTH: u32 = 1600;
/// Largest saved layout (project or group) once serialised.
pub const MAX_LAYOUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedState {
    pub version: u32,
    /// In sidebar order.
    pub projects: Vec<Project>,
    /// Only sessions shown in the sidebar. Closing one removes it.
    pub sessions: Vec<SavedSession>,
    pub settings: Settings,
    #[serde(default)]
    pub groups: Vec<Group>,
}

/// A named saved split layout. `layout` is the window's layout tree, opaque to the helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default = "empty_object")]
    pub layout: serde_json::Value,
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("layout must be a JSON object")]
    NotObject,
    #[error("layout is larger than 64 KB")]
    TooLarge,
    #[error("unknown id")]
    Unknown,
}

fn validate_layout(v: &serde_json::Value) -> Result<(), LayoutError> {
    if !v.is_object() {
        return Err(LayoutError::NotObject);
    }
    match serde_json::to_vec(v) {
        Ok(b) if b.len() <= MAX_LAYOUT_BYTES => Ok(()),
        _ => Err(LayoutError::TooLarge),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub shell_panel: ShellPanel,
    /// The project's saved split layout (opaque JSON object). `None` = never split.
    #[serde(default)]
    pub layout: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSession {
    /// The deck id: the app's own UUID for this entry, passed to Claude as CLAUDE_DECK_SESSION. Never changes.
    pub id: String,
    /// Claude's conversation to wake with --resume. Starts equal to `id`; changes
    /// after /clear or /resume inside the running Claude (spec §4.1).
    #[serde(default)]
    pub conversation_id: String,
    pub project_id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Set once the user renames it. The first prompt then never overwrites the name.
    #[serde(default)]
    pub renamed: bool,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub last_active_ms: i64,
    /// Claude has written a transcript for `conversation_id`, so waking can
    /// `--resume` it. False for a new session until its first turn (an untouched
    /// session writes none, spike 3); waking then uses `--session-id` instead.
    /// `/clear` writes one at once. Old files: true.
    #[serde(default = "default_true")]
    pub has_conversation: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Minutes before an Idle session sleeps. `None` (JSON null) = off.
    pub auto_sleep_minutes: Option<u32>,
    /// Full path to claude.exe. `None` = find it automatically.
    pub claude_path: Option<String>,
    pub window: Option<WindowPlacement>,
    pub shell: ShellKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShellKind {
    #[default]
    PowerShell,
    GitBash,
    Cmd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellPanel {
    pub open: bool,
    pub width: u32,
}

impl Default for ShellPanel {
    fn default() -> Self {
        Self {
            open: false,
            width: 480,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub struct LoadOutcome {
    pub state: SavedState,
    /// Set when the file could not be read as JSON and was moved here.
    pub corrupt_backup: Option<PathBuf>,
}

impl Default for SavedState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            projects: Vec::new(),
            sessions: Vec::new(),
            settings: Settings::default(),
            groups: Vec::new(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_sleep_minutes: Some(30),
            claude_path: None,
            window: None,
            shell: ShellKind::default(),
        }
    }
}

impl Settings {
    /// Keeps auto-sleep within 5–240 minutes (spec §4.2).
    pub fn normalize(&mut self) {
        self.auto_sleep_minutes = self
            .auto_sleep_minutes
            .map(|m| m.clamp(MIN_AUTO_SLEEP_MINUTES, MAX_AUTO_SLEEP_MINUTES));
    }
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Drops trailing slashes. "C:\" must stay "C:\": "C:" alone means "current folder on drive C".
fn clean_path(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    if trimmed.ends_with(':') {
        format!("{trimmed}\\")
    } else {
        trimmed.to_string()
    }
}

/// The folder name, with a drive root kept as "C:\".
fn project_name(path: &str) -> String {
    let clean = clean_path(path);
    Path::new(&clean)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(clean)
}

impl SavedState {
    /// Adds a project folder, or returns the existing one for the same folder.
    /// The caller checks that `path` is an existing folder.
    pub fn add_project(&mut self, path: &str) -> &Project {
        if let Some(i) = self.projects.iter().position(|p| same_path(&p.path, path)) {
            return &self.projects[i];
        }
        let clean = clean_path(path);
        let name = project_name(&clean);
        self.projects.push(Project {
            id: new_id(),
            path: clean,
            name,
            shell_panel: ShellPanel::default(),
            layout: None,
        });
        self.projects.last().expect("just pushed")
    }

    /// Stores (or clears, with `None`) a project's saved layout.
    pub fn set_project_layout(
        &mut self,
        project_id: &str,
        layout: Option<serde_json::Value>,
    ) -> Result<(), LayoutError> {
        if let Some(l) = &layout {
            validate_layout(l)?;
        }
        let p = self
            .projects
            .iter_mut()
            .find(|p| p.id == project_id)
            .ok_or(LayoutError::Unknown)?;
        p.layout = layout;
        Ok(())
    }

    /// Adds a group with an empty layout. A blank name becomes "Group N".
    pub fn add_group(&mut self, name: &str) -> &Group {
        let name = name.trim();
        let name = if name.is_empty() {
            format!("Group {}", self.groups.len() + 1)
        } else {
            name.to_string()
        };
        self.groups.push(Group {
            id: new_id(),
            name,
            layout: empty_object(),
        });
        self.groups.last().expect("just pushed")
    }

    /// A blank name is ignored (the old name stays).
    pub fn rename_group(&mut self, group_id: &str, name: &str) -> Result<(), LayoutError> {
        let g = self
            .groups
            .iter_mut()
            .find(|g| g.id == group_id)
            .ok_or(LayoutError::Unknown)?;
        let name = name.trim();
        if !name.is_empty() {
            g.name = name.to_string();
        }
        Ok(())
    }

    pub fn remove_group(&mut self, group_id: &str) -> Result<(), LayoutError> {
        let i = self
            .groups
            .iter()
            .position(|g| g.id == group_id)
            .ok_or(LayoutError::Unknown)?;
        self.groups.remove(i);
        Ok(())
    }

    pub fn set_group_layout(
        &mut self,
        group_id: &str,
        layout: serde_json::Value,
    ) -> Result<(), LayoutError> {
        validate_layout(&layout)?;
        let g = self
            .groups
            .iter_mut()
            .find(|g| g.id == group_id)
            .ok_or(LayoutError::Unknown)?;
        g.layout = layout;
        Ok(())
    }

    pub fn project(&self, project_id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == project_id)
    }

    /// Stores the project terminal panel state. Width is kept within 240..=1600.
    pub fn set_shell_panel(&mut self, project_id: &str, mut panel: ShellPanel) {
        panel.width = panel.width.clamp(MIN_SHELL_WIDTH, MAX_SHELL_WIDTH);
        if let Some(p) = self.projects.iter_mut().find(|p| p.id == project_id) {
            p.shell_panel = panel;
        }
    }

    /// Removes the project and its sessions from the app. Files are not touched.
    pub fn remove_project(&mut self, project_id: &str) {
        self.projects.retain(|p| p.id != project_id);
        self.sessions.retain(|s| s.project_id != project_id);
    }

    /// Adds a session with Claude's id. Adding the same id again keeps the first.
    pub fn add_session(&mut self, project_id: &str, id: String, now_ms: i64) -> &SavedSession {
        if let Some(i) = self.sessions.iter().position(|s| s.id == id) {
            return &self.sessions[i];
        }
        self.sessions.push(SavedSession {
            conversation_id: id.clone(),
            id,
            project_id: project_id.to_string(),
            name: None,
            renamed: false,
            created_ms: now_ms,
            last_active_ms: now_ms,
            has_conversation: false,
        });
        self.sessions.last().expect("just pushed")
    }

    /// Opens an old conversation ("Resume past session…") under a fresh deck id.
    /// If a session already shows that conversation, returns it. Either way the
    /// conversation exists on disk, so it can be resumed.
    pub fn add_past_session(
        &mut self,
        project_id: &str,
        conversation_id: &str,
        now_ms: i64,
    ) -> &SavedSession {
        if let Some(i) = self
            .sessions
            .iter()
            .position(|s| s.conversation_id == conversation_id)
        {
            self.sessions[i].has_conversation = true;
            return &self.sessions[i];
        }
        self.sessions.push(SavedSession {
            id: new_id(),
            conversation_id: conversation_id.to_string(),
            project_id: project_id.to_string(),
            name: None,
            renamed: false,
            created_ms: now_ms,
            last_active_ms: now_ms,
            has_conversation: true,
        });
        self.sessions.last().expect("just pushed")
    }

    /// Records the conversation Claude is now in (each SessionStart hook).
    pub fn set_conversation(&mut self, deck_id: &str, conversation_id: &str) {
        if let Some(s) = self.session_mut(deck_id) {
            s.conversation_id = conversation_id.to_string();
        }
    }

    /// Records whether the current conversation has been saved by Claude
    /// (a prompt was sent in it), so waking can `--resume` it.
    pub fn set_has_conversation(&mut self, deck_id: &str, has: bool) {
        if let Some(s) = self.session_mut(deck_id) {
            s.has_conversation = has;
        }
    }

    /// The conversations the sidebar already shows.
    pub fn conversation_ids(&self) -> std::collections::HashSet<String> {
        self.sessions
            .iter()
            .map(|s| s.conversation_id.clone())
            .collect()
    }

    pub fn set_settings(&mut self, mut settings: Settings) {
        settings.normalize();
        self.settings = settings;
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.sessions.retain(|s| s.id != session_id);
    }

    pub fn session(&self, session_id: &str) -> Option<&SavedSession> {
        self.sessions.iter().find(|s| s.id == session_id)
    }

    /// Names an unnamed session after its first prompt, cut to 40 characters.
    pub fn name_from_prompt(&mut self, session_id: &str, prompt: &str) {
        let Some(s) = self.session_mut(session_id) else {
            return;
        };
        if s.renamed || s.name.is_some() {
            return;
        }
        let text = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        // Claude sends "<task-notification>…" prompts itself; they are not the user's.
        if text.is_empty() || text.starts_with("<task-notification>") {
            return;
        }
        s.name = Some(if text.chars().count() > NAME_CHARS {
            let cut: String = text.chars().take(NAME_CHARS).collect();
            format!("{}…", cut.trim_end())
        } else {
            text
        });
    }

    pub fn rename(&mut self, session_id: &str, name: &str) {
        if let Some(s) = self.session_mut(session_id) {
            let name = name.trim();
            s.renamed = !name.is_empty();
            s.name = s.renamed.then(|| name.to_string());
        }
    }

    pub fn touch(&mut self, session_id: &str, now_ms: i64) {
        if let Some(s) = self.session_mut(session_id) {
            s.last_active_ms = now_ms;
        }
    }

    fn session_mut(&mut self, session_id: &str) -> Option<&mut SavedSession> {
        self.sessions.iter_mut().find(|s| s.id == session_id)
    }
}

fn with_path(e: io::Error, action: &str, path: &Path) -> io::Error {
    io::Error::new(e.kind(), format!("{action} {}: {e}", path.display()))
}

/// Loads the state. A missing file gives defaults. A file that is not valid
/// JSON for this format is moved to "<name>.corrupt-<now_ms>.json" and defaults are returned.
pub fn load(path: &Path, now_ms: i64) -> io::Result<LoadOutcome> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(LoadOutcome {
                state: SavedState::default(),
                corrupt_backup: None,
            })
        }
        Err(e) => return Err(with_path(e, "reading", path)),
    };
    let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    match serde_json::from_slice::<SavedState>(body) {
        Ok(mut state) => {
            state.settings.normalize();
            for s in &mut state.sessions {
                if s.conversation_id.is_empty() {
                    s.conversation_id = s.id.clone();
                }
            }
            for p in &mut state.projects {
                if p.name.is_empty() {
                    p.name = project_name(&p.path);
                }
            }
            Ok(LoadOutcome {
                state,
                corrupt_backup: None,
            })
        }
        Err(_) => {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("state");
            let backup = path.with_file_name(format!("{stem}.corrupt-{now_ms}.json"));
            fs::rename(path, &backup).map_err(|e| with_path(e, "moving aside", path))?;
            Ok(LoadOutcome {
                state: SavedState::default(),
                corrupt_backup: Some(backup),
            })
        }
    }
}

/// Writes to "<file>.tmp", flushes it to disk, then renames it over the old
/// file, so a crash never leaves a half-written state. Only the helper process writes this file.
pub fn save(path: &Path, state: &SavedState) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| with_path(e, "creating", dir))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let write = || -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&json)?;
        file.sync_all()
    };
    write().map_err(|e| with_path(e, "writing", &tmp))?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(with_path(e, "replacing", path));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID1: &str = "11111111-1111-4111-8111-111111111111";

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let out = load(&dir.path().join("state.json"), 1).unwrap();
        assert_eq!(out.state, SavedState::default());
        assert_eq!(out.state.settings.auto_sleep_minutes, Some(30));
        assert!(out.corrupt_backup.is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("state.json");
        let mut state = SavedState::default();
        let pid = state.add_project(r"C:\Work\App").id.clone();
        state.add_session(&pid, ID1.into(), 5);
        state.settings.auto_sleep_minutes = None;
        save(&path, &state).unwrap();
        assert_eq!(load(&path, 6).unwrap().state, state);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, "{ not json").unwrap();
        let out = load(&path, 1234).unwrap();
        assert_eq!(out.state, SavedState::default());
        let backup = out.corrupt_backup.unwrap();
        assert_eq!(backup, dir.path().join("state.corrupt-1234.json"));
        assert_eq!(fs::read_to_string(backup).unwrap(), "{ not json");
        assert!(!path.exists());
    }

    #[test]
    fn auto_sleep_stays_between_5_and_240_minutes_and_null_means_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"settings":{"auto_sleep_minutes":1}}"#).unwrap();
        assert_eq!(
            load(&path, 1).unwrap().state.settings.auto_sleep_minutes,
            Some(5)
        );
        fs::write(&path, r#"{"settings":{"auto_sleep_minutes":999}}"#).unwrap();
        assert_eq!(
            load(&path, 1).unwrap().state.settings.auto_sleep_minutes,
            Some(240)
        );
        fs::write(&path, r#"{"settings":{"auto_sleep_minutes":null}}"#).unwrap();
        assert_eq!(
            load(&path, 1).unwrap().state.settings.auto_sleep_minutes,
            None
        );
    }

    #[test]
    fn adding_the_same_folder_twice_keeps_one_project() {
        let mut s = SavedState::default();
        let a = s.add_project(r"C:\Work\App").id.clone();
        let b = s.add_project("c:/work/app/").id.clone();
        assert_eq!(a, b);
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.projects[0].name, "App");
    }

    #[test]
    fn removing_a_project_removes_its_sessions() {
        let mut s = SavedState::default();
        let a = s.add_project(r"C:\Work\A").id.clone();
        let b = s.add_project(r"C:\Work\B").id.clone();
        s.add_session(&a, "s1".into(), 1);
        s.add_session(&b, "s2".into(), 1);
        s.remove_project(&a);
        assert_eq!(s.projects.len(), 1);
        assert_eq!(
            s.sessions.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
            vec!["s2"]
        );
    }

    #[test]
    fn adding_an_existing_session_twice_keeps_one() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.add_session(&p, "s1".into(), 2);
        assert_eq!(s.sessions.len(), 1);
        s.remove_session("s1");
        assert!(s.sessions.is_empty());
    }

    #[test]
    fn first_prompt_names_a_session_once_and_rename_wins() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.name_from_prompt(
            "s1",
            "  fix the\ninvoice   total when a discount is applied twice  ",
        );
        assert_eq!(
            s.session("s1").unwrap().name.as_deref(),
            Some("fix the invoice total when a discount is…")
        );
        s.name_from_prompt("s1", "a later prompt");
        assert_eq!(
            s.session("s1").unwrap().name.as_deref(),
            Some("fix the invoice total when a discount is…")
        );
        s.rename("s1", "Invoice bug");
        let sess = s.session("s1").unwrap();
        assert_eq!(sess.name.as_deref(), Some("Invoice bug"));
        assert!(sess.renamed);
    }

    #[test]
    fn short_prompts_are_kept_whole_and_blank_ones_ignored() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.name_from_prompt("s1", "   ");
        assert_eq!(s.session("s1").unwrap().name, None);
        // Claude sends this itself when background work reports back.
        s.name_from_prompt(
            "s1",
            "<task-notification><status>completed</status></task-notification>",
        );
        assert_eq!(s.session("s1").unwrap().name, None);
        s.name_from_prompt("s1", "hello");
        assert_eq!(s.session("s1").unwrap().name.as_deref(), Some("hello"));
    }

    #[test]
    fn touch_updates_last_active() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.touch("s1", 99);
        assert_eq!(s.session("s1").unwrap().last_active_ms, 99);
    }

    #[test]
    fn a_minimal_session_entry_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"projects":[{"id":"p","path":"C:\\Work\\A"}],"sessions":[{"id":"s1","project_id":"p"}]}"#).unwrap();
        let out = load(&path, 1).unwrap();
        assert!(out.corrupt_backup.is_none());
        let s = out.state.session("s1").unwrap();
        assert_eq!((s.name.clone(), s.renamed, s.created_ms), (None, false, 0));
    }

    #[test]
    fn a_utf8_bom_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(
            &path,
            b"\xEF\xBB\xBF{\"settings\":{\"auto_sleep_minutes\":45}}",
        )
        .unwrap();
        let out = load(&path, 1).unwrap();
        assert!(out.corrupt_backup.is_none());
        assert_eq!(out.state.settings.auto_sleep_minutes, Some(45));
    }

    #[test]
    fn a_drive_root_keeps_its_backslash() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\").clone();
        assert_eq!((p.path.as_str(), p.name.as_str()), (r"C:\", r"C:\"));
    }

    #[test]
    fn a_blank_rename_clears_the_name() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.rename("s1", "Bug");
        s.rename("s1", "   ");
        let sess = s.session("s1").unwrap();
        assert_eq!((sess.name.clone(), sess.renamed), (None, false));
    }

    #[test]
    fn read_errors_name_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = load(dir.path(), 1).unwrap_err(); // a folder, not a file
        assert!(
            err.to_string().contains(&dir.path().display().to_string()),
            "{err}"
        );
    }

    #[test]
    fn new_ids_are_uuids() {
        assert!(uuid::Uuid::parse_str(&new_id()).is_ok());
    }

    #[test]
    fn an_old_session_loads_with_conversation_id_equal_to_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"projects":[{"id":"p","path":"C:\\Work\\A"}],"sessions":[{"id":"s1","project_id":"p"}]}"#).unwrap();
        let out = load(&path, 1).unwrap();
        assert_eq!(out.state.session("s1").unwrap().conversation_id, "s1");
    }

    #[test]
    fn a_project_with_an_empty_name_loads_with_its_folder_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"projects":[{"id":"p","path":"C:\\Work\\App\\","name":""},{"id":"q","path":"C:\\"}]}"#).unwrap();
        let out = load(&path, 1).unwrap();
        assert_eq!(out.state.projects[0].name, "App");
        assert_eq!(out.state.projects[1].name, r"C:\");
    }

    #[test]
    fn add_session_starts_with_conversation_equal_to_deck_id() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        let sess = s.add_session(&p, ID1.into(), 1);
        assert_eq!(sess.conversation_id, ID1);
    }

    #[test]
    fn a_past_session_gets_a_fresh_deck_id_and_is_added_once() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        let first = s.add_past_session(&p, ID1, 7).clone();
        assert_ne!(first.id, ID1);
        assert!(uuid::Uuid::parse_str(&first.id).is_ok());
        assert_eq!(first.conversation_id, ID1);
        assert_eq!(first.project_id, p);
        assert_eq!((first.created_ms, first.last_active_ms), (7, 7));
        let again = s.add_past_session(&p, ID1, 8).clone();
        assert_eq!(again, first);
        assert_eq!(s.sessions.len(), 1);
    }

    #[test]
    fn set_conversation_updates_only_that_session() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.add_session(&p, "s2".into(), 1);
        s.set_conversation("s1", "c9");
        assert_eq!(s.session("s1").unwrap().conversation_id, "c9");
        assert_eq!(s.session("s1").unwrap().id, "s1");
        assert_eq!(s.session("s2").unwrap().conversation_id, "s2");
        s.set_conversation("missing", "x");
        assert_eq!(s.sessions.len(), 2);
    }

    #[test]
    fn conversation_ids_returns_the_set() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.add_session(&p, "s1".into(), 1);
        s.add_past_session(&p, "c2", 1);
        s.set_conversation("s1", "c1");
        let want: std::collections::HashSet<String> =
            ["c1".to_string(), "c2".to_string()].into_iter().collect();
        assert_eq!(s.conversation_ids(), want);
    }

    #[test]
    fn set_settings_normalizes_before_storing() {
        let mut s = SavedState::default();
        s.set_settings(Settings {
            auto_sleep_minutes: Some(1),
            ..Default::default()
        });
        assert_eq!(s.settings.auto_sleep_minutes, Some(5));
    }

    #[test]
    fn an_old_file_loads_with_powershell_and_a_closed_480_panel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(
            &path,
            r#"{"projects":[{"id":"p","path":"C:\\Work\\A"}],"settings":{"auto_sleep_minutes":30}}"#,
        )
        .unwrap();
        let st = load(&path, 1).unwrap().state;
        assert_eq!(st.settings.shell, ShellKind::PowerShell);
        assert_eq!(
            st.project("p").unwrap().shell_panel,
            ShellPanel {
                open: false,
                width: 480
            }
        );
    }

    #[test]
    fn the_shell_setting_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut st = SavedState::default();
        st.set_settings(Settings {
            shell: ShellKind::GitBash,
            ..Default::default()
        });
        save(&path, &st).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("git_bash"));
        assert_eq!(
            load(&path, 1).unwrap().state.settings.shell,
            ShellKind::GitBash
        );
    }

    #[test]
    fn set_shell_panel_clamps_the_width() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        s.set_shell_panel(
            &p,
            ShellPanel {
                open: true,
                width: 100,
            },
        );
        assert_eq!(
            s.project(&p).unwrap().shell_panel,
            ShellPanel {
                open: true,
                width: 240
            }
        );
        s.set_shell_panel(
            &p,
            ShellPanel {
                open: true,
                width: 9999,
            },
        );
        assert_eq!(s.project(&p).unwrap().shell_panel.width, 1600);
        s.set_shell_panel(
            "missing",
            ShellPanel {
                open: false,
                width: 500,
            },
        );
        assert_eq!(s.projects.len(), 1);
    }

    #[test]
    fn has_conversation_starts_false_for_new_true_for_past_and_old_files() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        assert!(!s.add_session(&p, "s1".into(), 1).has_conversation);
        s.set_has_conversation("s1", true);
        assert!(s.session("s1").unwrap().has_conversation);
        s.set_has_conversation("s1", false);
        assert!(!s.session("s1").unwrap().has_conversation);
        s.set_has_conversation("missing", true);
        assert!(s.add_past_session(&p, "c2", 1).has_conversation);
        // Opening a conversation the sidebar already shows: it exists on disk.
        assert!(s.add_past_session(&p, "s1", 2).has_conversation);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"projects":[{"id":"p","path":"C:\\Work\\A"}],"sessions":[{"id":"s1","project_id":"p"}]}"#).unwrap();
        assert!(
            load(&path, 1)
                .unwrap()
                .state
                .session("s1")
                .unwrap()
                .has_conversation
        );
        save(&path, &s).unwrap();
        assert_eq!(load(&path, 1).unwrap().state, s);
    }

    #[test]
    fn project_finds_by_id() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        assert_eq!(s.project(&p).unwrap().path, r"C:\Work\A");
        assert!(s.project("nope").is_none());
    }

    #[test]
    fn an_old_file_loads_without_layouts_or_groups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"projects":[{"id":"p","path":"C:\\Work\\A"}]}"#).unwrap();
        let st = load(&path, 1).unwrap().state;
        assert_eq!(st.project("p").unwrap().layout, None);
        assert!(st.groups.is_empty());
    }

    #[test]
    fn layouts_and_groups_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        let lay = serde_json::json!({"type":"pane","id":"p1","tabs":[],"active":0});
        s.set_project_layout(&p, Some(lay.clone())).unwrap();
        let g1 = s.add_group("Billing").id.clone();
        s.add_group("Ops");
        s.set_group_layout(&g1, lay.clone()).unwrap();
        save(&path, &s).unwrap();
        let back = load(&path, 1).unwrap().state;
        assert_eq!(back, s);
        assert_eq!(back.project(&p).unwrap().layout, Some(lay.clone()));
        assert_eq!(back.groups.len(), 2);
        assert_eq!(back.groups[0].layout, lay);
        s.set_project_layout(&p, None).unwrap();
        assert_eq!(s.project(&p).unwrap().layout, None);
    }

    #[test]
    fn a_group_without_a_layout_loads_with_an_empty_object() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"groups":[{"id":"g","name":"N"}]}"#).unwrap();
        let st = load(&path, 1).unwrap().state;
        assert_eq!(st.groups[0].layout, serde_json::json!({}));
    }

    #[test]
    fn group_names_are_trimmed_and_blank_ones_numbered() {
        let mut s = SavedState::default();
        assert_eq!(s.add_group("  ").name, "Group 1");
        assert_eq!(s.add_group("").name, "Group 2");
        let id = s.add_group("  Billing ").id.clone();
        assert_eq!(s.groups[2].name, "Billing");
        assert_eq!(s.groups[0].layout, serde_json::json!({}));
        s.rename_group(&id, " Pay ").unwrap();
        assert_eq!(s.groups[2].name, "Pay");
        s.rename_group(&id, "   ").unwrap();
        assert_eq!(s.groups[2].name, "Pay");
        assert_eq!(s.rename_group("nope", "x"), Err(LayoutError::Unknown));
        s.remove_group(&id).unwrap();
        assert_eq!(s.groups.len(), 2);
        assert_eq!(s.remove_group(&id), Err(LayoutError::Unknown));
    }

    #[test]
    fn bad_layouts_are_refused_and_change_nothing() {
        let mut s = SavedState::default();
        let p = s.add_project(r"C:\Work\A").id.clone();
        let g = s.add_group("G").id.clone();
        let arr = serde_json::json!([1]);
        assert_eq!(
            s.set_group_layout(&g, arr.clone()),
            Err(LayoutError::NotObject)
        );
        assert_eq!(
            s.set_project_layout(&p, Some(arr)),
            Err(LayoutError::NotObject)
        );
        let big = serde_json::json!({"x": "a".repeat(70 * 1024)});
        assert_eq!(
            s.set_group_layout(&g, big.clone()),
            Err(LayoutError::TooLarge)
        );
        assert_eq!(
            s.set_project_layout(&p, Some(big)),
            Err(LayoutError::TooLarge)
        );
        assert_eq!(s.groups[0].layout, serde_json::json!({}));
        assert_eq!(s.project(&p).unwrap().layout, None);
        let ok = serde_json::json!({"x": 1});
        assert_eq!(
            s.set_group_layout("nope", ok.clone()),
            Err(LayoutError::Unknown)
        );
        assert_eq!(
            s.set_project_layout("nope", Some(ok)),
            Err(LayoutError::Unknown)
        );
        assert_eq!(
            s.set_project_layout("nope", None),
            Err(LayoutError::Unknown)
        );
    }
}
