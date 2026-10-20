//! Finds a project's Claude conversations under ~/.claude/projects (spec §4.7).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const HEAD_BYTES: u64 = 32 * 1024;
const TAIL_BYTES: u64 = 64 * 1024;

/// A conversation that can be reopened from "Resume past session…".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PastSession {
    pub id: String,
    pub last_prompt: Option<String>,
    pub modified_ms: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TranscriptSummary {
    pub cwd: Option<String>,
    pub last_prompt: Option<String>,
}

/// Claude's folder name for a project: every character that is not an ASCII
/// letter or digit becomes '-'.
pub fn encode_project_dir(project_path: &str) -> String {
    project_path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Windows paths: compare without case, with either slash, without a trailing slash.
pub fn same_path(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

/// The folder under `projects_root` for this project, ignoring case.
pub fn find_project_dir(projects_root: &Path, project_path: &str) -> Option<PathBuf> {
    let wanted = encode_project_dir(project_path).to_ascii_lowercase();
    fs::read_dir(projects_root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .find(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == wanted)
        .map(|e| e.path())
}

/// Conversations for `project_path`, newest first. Skips ids in `exclude`
/// (already open in the app) and files whose `cwd` is a different folder.
pub fn list_past_sessions(
    projects_root: &Path,
    project_path: &str,
    exclude: &HashSet<String>,
) -> Vec<PastSession> {
    let Some(dir) = find_project_dir(projects_root, project_path) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if exclude.contains(&id) {
            continue;
        }
        let Ok(summary) = summarize_file(&path) else {
            continue;
        };
        if let Some(cwd) = &summary.cwd {
            if !same_path(cwd, project_path) {
                continue;
            }
        }
        let modified_ms = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        out.push(PastSession {
            id,
            last_prompt: summary.last_prompt,
            modified_ms,
        });
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.modified_ms));
    out
}

/// Reads only the head and tail of a transcript, then summarizes them.
pub fn summarize_file(path: &Path) -> io::Result<TranscriptSummary> {
    Ok(summarize_transcript(&read_head_and_tail(path)?))
}

/// The first `cwd` and the last non-empty `lastPrompt`. Lines that are not
/// valid JSON (for example, cut at the head/tail boundary) are skipped.
pub fn summarize_transcript(text: &str) -> TranscriptSummary {
    let mut s = TranscriptSummary::default();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if s.cwd.is_none() {
            if let Some(cwd) = v.get("cwd").and_then(Value::as_str) {
                s.cwd = Some(cwd.to_string());
            }
        }
        if v.get("type").and_then(Value::as_str) == Some("last-prompt") {
            if let Some(p) = v.get("lastPrompt").and_then(Value::as_str) {
                if !p.is_empty() {
                    s.last_prompt = Some(p.to_string());
                }
            }
        }
    }
    s
}

fn read_head_and_tail(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut bytes = Vec::new();
    if len <= HEAD_BYTES + TAIL_BYTES {
        file.read_to_end(&mut bytes)?;
    } else {
        bytes.resize(HEAD_BYTES as usize, 0);
        file.read_exact(&mut bytes)?;
        bytes.push(b'\n');
        file.seek(SeekFrom::End(-(TAIL_BYTES as i64)))?;
        file.read_to_end(&mut bytes)?;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::time::{Duration, SystemTime};

    #[test]
    fn encodes_like_claude_does() {
        assert_eq!(
            encode_project_dir(r"C:\Users\Chamoux\Desktop\CodeHive"),
            "C--Users-Chamoux-Desktop-CodeHive"
        );
        assert_eq!(
            encode_project_dir(r"c:\Users\Chamoux\Desktop\New folder (2)"),
            "c--Users-Chamoux-Desktop-New-folder--2-"
        );
        assert_eq!(
            encode_project_dir(r"C:\Users\Chamoux\.claude\skills"),
            "C--Users-Chamoux--claude-skills"
        );
    }

    #[test]
    fn same_path_ignores_case_slashes_and_trailing_slash() {
        assert!(same_path(r"C:\Users\Me\App", "c:/users/me/app/"));
        assert!(!same_path(r"C:\Users\Me\App", r"C:\Users\Me\App2"));
    }

    #[test]
    fn finds_the_folder_whatever_the_drive_letter_case() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("c--Users-Me-Desktop-App")).unwrap();
        let found = find_project_dir(root.path(), r"C:\Users\Me\Desktop\App").unwrap();
        assert_eq!(found.file_name().unwrap(), "c--Users-Me-Desktop-App");
        assert!(find_project_dir(root.path(), r"C:\Users\Me\Desktop\Nope").is_none());
    }

    fn write_transcript(dir: &Path, id: &str, cwd: &str, prompt: &str, age_secs: u64) {
        let path = dir.join(format!("{id}.jsonl"));
        let cwd = serde_json::to_string(cwd).unwrap();
        let prompt = serde_json::to_string(prompt).unwrap();
        fs::write(
            &path,
            format!(
                "{{\"type\":\"last-prompt\",\"sessionId\":\"{id}\"}}\n{{\"type\":\"user\",\"cwd\":{cwd}}}\n{{\"type\":\"last-prompt\",\"lastPrompt\":{prompt}}}\n"
            ),
        )
        .unwrap();
        let when = SystemTime::now() - Duration::from_secs(age_secs);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    #[test]
    fn lists_past_sessions_newest_first_skipping_open_and_foreign_ones() {
        let root = tempfile::tempdir().unwrap();
        let project = r"C:\Users\Me\Desktop\My App";
        let dir = root.path().join("c--Users-Me-Desktop-My-App");
        fs::create_dir(&dir).unwrap();
        write_transcript(&dir, "old", project, "first idea", 300);
        write_transcript(&dir, "new", project, "second idea", 10);
        write_transcript(&dir, "open-already", project, "skip me", 5);
        // "My-App" encodes to the same folder name as "My App"; its cwd tells them apart.
        write_transcript(
            &dir,
            "other-folder",
            r"C:\Users\Me\Desktop\My-App",
            "not this project",
            1,
        );
        fs::write(dir.join("notes.txt"), "ignored").unwrap();

        let exclude = HashSet::from(["open-already".to_string()]);
        let got = list_past_sessions(root.path(), project, &exclude);
        let ids: Vec<&str> = got.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["new", "old"]);
        assert_eq!(got[0].last_prompt.as_deref(), Some("second idea"));
        assert!(got[0].modified_ms > got[1].modified_ms);
    }

    #[test]
    fn reads_cwd_and_latest_prompt_from_head_and_tail_of_a_big_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.jsonl");
        let mut text = String::from(
            "{\"type\":\"user\",\"cwd\":\"C:\\\\Work\\\\App\"}\n{\"type\":\"last-prompt\",\"lastPrompt\":\"early\"}\n",
        );
        let filler = format!(
            "{{\"type\":\"assistant\",\"text\":\"{}\"}}\n",
            "x".repeat(1000)
        );
        for _ in 0..500 {
            text.push_str(&filler); // about 500 KB: bigger than head + tail
        }
        text.push_str("{\"type\":\"last-prompt\",\"lastPrompt\":\"latest\"}\n{\"type\":\"last-prompt\",\"leafUuid\":\"x\"}\n");
        fs::write(&path, text).unwrap();

        let s = summarize_file(&path).unwrap();
        assert_eq!(s.cwd.as_deref(), Some(r"C:\Work\App"));
        assert_eq!(s.last_prompt.as_deref(), Some("latest"));
    }

    #[test]
    fn missing_root_gives_nothing() {
        let got = list_past_sessions(
            Path::new(r"C:\does\not\exist"),
            r"C:\Work\App",
            &HashSet::new(),
        );
        assert!(got.is_empty());
    }
}
