//! Runs git for the Changes tab and returns GitResult.

use std::io::ErrorKind;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use deck_core::git::{parse_porcelain_z, strip_prefix, summarize, DEFAULT_CAP};
use deck_proto::messages::GitResult;

/// Windows flag: do not open a console window for the child process.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Runs `git <args>` in `project`. Returns stdout on success, or the GitResult
/// to report on failure.
fn run_git(project: &Path, args: &[&str]) -> Result<Vec<u8>, GitResult> {
    let output = Command::new("git")
        .args(args)
        .current_dir(project)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                GitResult::GitMissing
            } else {
                GitResult::Error {
                    message: e.to_string(),
                }
            }
        })?;

    if output.status.success() {
        return Ok(output.stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.to_lowercase().contains("not a git repository") {
        return Err(GitResult::NotGit);
    }
    let message = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("git exited with {}", output.status));
    Err(GitResult::Error { message })
}

/// Runs `git rev-parse --show-prefix`, then
/// `git status --porcelain=v1 -z --untracked-files=all -- .` in `project`.
/// The project folder may be a subfolder of the repo, so paths are made
/// relative to it.
pub fn git_status(project: &Path) -> GitResult {
    let prefix = match run_git(project, &["rev-parse", "--show-prefix"]) {
        Ok(out) => String::from_utf8_lossy(&out).trim().to_owned(),
        Err(result) => return result,
    };
    let status = match run_git(
        project,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ],
    ) {
        Ok(out) => out,
        Err(result) => return result,
    };

    let changes = strip_prefix(parse_porcelain_z(&status), &prefix);
    GitResult::Ok {
        list: summarize(changes, DEFAULT_CAP),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deck_core::git::{Change, ChangeKind, ChangeList};
    use std::path::Path;
    use std::process::Command;

    /// Runs git in `dir` and panics if it fails.
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git must be installed for these tests");
        assert!(status.success(), "git {args:?} failed in {}", dir.display());
    }

    /// Makes a repo with one committed file, `a.txt`.
    fn repo_with_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q"]);
        std::fs::write(p.join("a.txt"), "one\n").unwrap();
        git(p, &["add", "a.txt"]);
        git(
            p,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "init",
            ],
        );
        dir
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
    fn root_repo_lists_modified_and_untracked_files() {
        let dir = repo_with_commit();
        let p = dir.path();
        std::fs::write(p.join("a.txt"), "two\n").unwrap();
        std::fs::write(p.join("b.txt"), "new\n").unwrap();

        let result = git_status(p);

        assert_eq!(
            result,
            GitResult::Ok {
                list: ChangeList {
                    items: vec![
                        change(ChangeKind::Created, "b.txt"),
                        change(ChangeKind::Modified, "a.txt"),
                    ],
                    total: 2,
                },
            }
        );
    }

    #[test]
    fn subfolder_project_lists_only_its_files_relative_to_itself() {
        let dir = repo_with_commit();
        let p = dir.path();
        std::fs::create_dir_all(p.join("app/web")).unwrap();
        std::fs::create_dir_all(p.join("other")).unwrap();
        std::fs::write(p.join("app/web/c.txt"), "c\n").unwrap();
        std::fs::write(p.join("other/d.txt"), "d\n").unwrap();

        let result = git_status(&p.join("app/web"));

        assert_eq!(
            result,
            GitResult::Ok {
                list: ChangeList {
                    items: vec![change(ChangeKind::Created, "c.txt")],
                    total: 1,
                },
            }
        );
    }

    #[test]
    fn plain_folder_that_is_not_a_repo_gives_not_git() {
        // %TEMP% should not sit inside any repo. If it ever does, this test will
        // report Ok instead of NotGit, and the cause is the parent repo.
        let dir = tempfile::tempdir().expect("tempdir");

        assert_eq!(git_status(dir.path()), GitResult::NotGit);
    }

    #[test]
    fn clean_repo_gives_empty_list() {
        let dir = repo_with_commit();

        assert_eq!(
            git_status(dir.path()),
            GitResult::Ok {
                list: ChangeList {
                    items: vec![],
                    total: 0,
                },
            }
        );
    }
}
