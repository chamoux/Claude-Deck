//! Parses `git status --porcelain=v1 -z --untracked-files=all` for the Changes tab (spec §6).

use serde::{Deserialize, Serialize};

/// Lines shown before "and N more" (spec §6).
pub const DEFAULT_CAP: usize = 500;

/// Order here is the order of the groups in the Changes tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub kind: ChangeKind,
    /// Relative to the repo root, with forward slashes.
    pub path: String,
    /// The previous path, for renames only.
    pub old_path: Option<String>,
    /// A merge conflict, shown as Modified with the word "conflict".
    pub conflict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeList {
    /// Sorted by kind, then path. At most the cap.
    pub items: Vec<Change>,
    /// How many changes there were before the cap.
    pub total: usize,
}

/// Parses the output of `git status --porcelain=v1 -z --untracked-files=all`.
/// Each entry is "XY path\0". Renames and copies are followed by "old path\0".
pub fn parse_porcelain_z(out: &[u8]) -> Vec<Change> {
    let text = String::from_utf8_lossy(out);
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut changes = Vec::new();
    while let Some(entry) = fields.next() {
        if entry.len() < 4 || !entry.is_char_boundary(3) {
            continue;
        }
        let (code, path) = entry.split_at(3);
        let mut chars = code.chars();
        let x = chars.next().unwrap_or(' ');
        let y = chars.next().unwrap_or(' ');
        let old_path = if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            fields.next().map(str::to_string)
        } else {
            None
        };
        if let Some(change) = classify(x, y, path.to_string(), old_path) {
            changes.push(change);
        }
    }
    changes
}

fn classify(x: char, y: char, path: String, old_path: Option<String>) -> Option<Change> {
    let conflict = matches!(
        (x, y),
        ('D', 'D') | ('A', 'U') | ('U', 'D') | ('U', 'A') | ('D', 'U') | ('A', 'A') | ('U', 'U')
    );
    let kind = if conflict {
        ChangeKind::Modified
    } else if x == '?' && y == '?' {
        ChangeKind::Created
    } else if x == '!' {
        return None;
    } else if x == 'R' || y == 'R' {
        ChangeKind::Renamed
    } else if x == 'C' || y == 'C' {
        ChangeKind::Created
    } else if x == 'A' && y == 'D' {
        // Added, then deleted again before any commit: nothing to show.
        return None;
    } else if x == 'D' || y == 'D' {
        ChangeKind::Deleted
    } else if x == 'A' || y == 'A' {
        ChangeKind::Created
    } else if matches!(x, 'M' | 'T') || matches!(y, 'M' | 'T') {
        ChangeKind::Modified
    } else {
        return None;
    };
    let old_path = if kind == ChangeKind::Renamed {
        old_path
    } else {
        None
    };
    Some(Change {
        kind,
        path,
        old_path,
        conflict,
    })
}

/// Sorts by kind, then path, and keeps at most `cap` items.
pub fn summarize(mut changes: Vec<Change>, cap: usize) -> ChangeList {
    changes.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.path.cmp(&b.path)));
    let total = changes.len();
    changes.truncate(cap);
    ChangeList {
        items: changes,
        total,
    }
}

/// "app/Services/InvoiceService.php" → ("InvoiceService.php", "app/Services").
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((folder, name)) => (name, folder),
        None => (path, ""),
    }
}

/// Makes repo-root paths relative to the project folder when the project is a
/// subfolder of the repo (spec §6). `prefix` is the output of
/// `git rev-parse --show-prefix`, such as "app/web/", or empty at the repo root.
/// A path outside the folder (the old side of a rename) gets "../" segments.
pub fn strip_prefix(changes: Vec<Change>, prefix: &str) -> Vec<Change> {
    let prefix = prefix.trim().trim_end_matches('/');
    if prefix.is_empty() {
        return changes;
    }
    let base: Vec<&str> = prefix.split('/').collect();
    changes
        .into_iter()
        .map(|c| Change {
            path: relative_to(&base, &c.path),
            old_path: c.old_path.as_deref().map(|p| relative_to(&base, p)),
            ..c
        })
        .collect()
}

fn relative_to(base: &[&str], path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let shared = base
        .iter()
        .zip(&parts)
        .take_while(|(a, b)| a == b)
        .count()
        .min(parts.len().saturating_sub(1));
    let mut out: Vec<&str> = vec![".."; base.len() - shared];
    out.extend(&parts[shared..]);
    out.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_conflict_code_is_modified_with_conflict() {
        let out = b"DD a\0AU b\0UD c\0UA d\0DU e\0AA f\0UU g\0";
        let got = parse_porcelain_z(out);
        assert_eq!(got.len(), 7);
        assert!(
            got.iter()
                .all(|c| c.kind == ChangeKind::Modified && c.conflict),
            "{got:?}"
        );
    }

    fn change(kind: ChangeKind, path: &str) -> Change {
        Change {
            kind,
            path: path.into(),
            old_path: None,
            conflict: false,
        }
    }

    #[test]
    fn maps_every_status_code() {
        let out = b"?? new.txt\0 M app/a.php\0M  app/b.php\0D  old.php\0 D gone.php\0R  new/name.php\0old/name.php\0UU both.php\0A  added.php\0AM added2.php\0AD ghost.php\0 T link\0C  copy.php\0orig.php\0";
        assert_eq!(
            parse_porcelain_z(out),
            vec![
                change(ChangeKind::Created, "new.txt"),
                change(ChangeKind::Modified, "app/a.php"),
                change(ChangeKind::Modified, "app/b.php"),
                change(ChangeKind::Deleted, "old.php"),
                change(ChangeKind::Deleted, "gone.php"),
                Change {
                    kind: ChangeKind::Renamed,
                    path: "new/name.php".into(),
                    old_path: Some("old/name.php".into()),
                    conflict: false,
                },
                Change {
                    kind: ChangeKind::Modified,
                    path: "both.php".into(),
                    old_path: None,
                    conflict: true
                },
                change(ChangeKind::Created, "added.php"),
                change(ChangeKind::Created, "added2.php"),
                change(ChangeKind::Modified, "link"),
                change(ChangeKind::Created, "copy.php"),
            ]
        );
    }

    #[test]
    fn paths_with_spaces_and_accents_survive() {
        let out = "?? docs/my notes é.md\0".as_bytes();
        assert_eq!(
            parse_porcelain_z(out),
            vec![change(ChangeKind::Created, "docs/my notes é.md")]
        );
    }

    #[test]
    fn empty_and_short_entries_are_skipped() {
        assert!(parse_porcelain_z(b"").is_empty());
        assert!(parse_porcelain_z(b"?? \0\0").is_empty());
    }

    #[test]
    fn summarize_sorts_by_kind_then_path_and_caps() {
        let list = summarize(
            vec![
                change(ChangeKind::Deleted, "z.txt"),
                change(ChangeKind::Created, "b.txt"),
                change(ChangeKind::Modified, "m.txt"),
                change(ChangeKind::Created, "a.txt"),
            ],
            3,
        );
        assert_eq!(list.total, 4);
        assert_eq!(
            list.items,
            vec![
                change(ChangeKind::Created, "a.txt"),
                change(ChangeKind::Created, "b.txt"),
                change(ChangeKind::Modified, "m.txt"),
            ]
        );
    }

    #[test]
    fn split_path_gives_name_and_folder() {
        assert_eq!(
            split_path("app/Services/InvoiceService.php"),
            ("InvoiceService.php", "app/Services")
        );
        assert_eq!(split_path("README.md"), ("README.md", ""));
    }

    #[test]
    fn strip_prefix_makes_paths_relative_to_the_project_folder() {
        let changes = vec![
            change(ChangeKind::Modified, "app/web/src/a.ts"),
            Change {
                kind: ChangeKind::Renamed,
                path: "app/web/new.ts".into(),
                old_path: Some("lib/old.ts".into()),
                conflict: false,
            },
            Change {
                kind: ChangeKind::Renamed,
                path: "app/web/b.ts".into(),
                old_path: Some("app/b.ts".into()),
                conflict: false,
            },
        ];
        let got = strip_prefix(changes.clone(), "app/web/");
        assert_eq!(got[0].path, "src/a.ts");
        assert_eq!(
            (got[1].path.as_str(), got[1].old_path.as_deref()),
            ("new.ts", Some("../../lib/old.ts"))
        );
        assert_eq!(
            (got[2].path.as_str(), got[2].old_path.as_deref()),
            ("b.ts", Some("../b.ts"))
        );
        assert_eq!(
            strip_prefix(changes.clone(), "app/web"),
            got,
            "a missing trailing slash works too"
        );
        assert_eq!(
            strip_prefix(changes.clone(), ""),
            changes,
            "the repo root changes nothing"
        );
        assert_eq!(
            strip_prefix(changes.clone(), " \n"),
            changes,
            "rev-parse output may end in a newline"
        );
    }

    #[test]
    fn intent_to_add_and_mixed_codes() {
        let out = b" A planned.txt\0MD half.php\0RM moved.php\0was.php\0!! ignored.log\0";
        assert_eq!(
            parse_porcelain_z(out),
            vec![
                change(ChangeKind::Created, "planned.txt"),
                change(ChangeKind::Deleted, "half.php"),
                Change {
                    kind: ChangeKind::Renamed,
                    path: "moved.php".into(),
                    old_path: Some("was.php".into()),
                    conflict: false,
                },
            ]
        );
    }
}
