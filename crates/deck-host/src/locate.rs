//! Finds claude.exe, the shell exe, and the %LOCALAPPDATA%\ClaudeDeck paths.

use deck_core::store::ShellKind;
use std::path::{Path, PathBuf};

/// %LOCALAPPDATA%\ClaudeDeck, created if missing. Error if LOCALAPPDATA is unset.
pub fn app_dir() -> std::io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA is not set")
    })?;
    let dir = PathBuf::from(base).join("ClaudeDeck");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// <app_dir>\state.json
pub fn state_path(app_dir: &Path) -> PathBuf {
    app_dir.join("state.json")
}

/// <app_dir>\hooks.json
pub fn hooks_path(app_dir: &Path) -> PathBuf {
    app_dir.join("hooks.json")
}

/// <app_dir>\logs
pub fn logs_dir(app_dir: &Path) -> PathBuf {
    app_dir.join("logs")
}

/// The setting if it names an existing file, else claude.exe found on PATH, else
/// %USERPROFILE%\.local\bin\claude.exe if it exists, else None.
pub fn find_claude(setting: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = setting.map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(path) = find_on_path("claude.exe") {
        return Some(path);
    }
    let home = std::env::var_os("USERPROFILE")?;
    let path = Path::new(&home)
        .join(".local")
        .join("bin")
        .join("claude.exe");
    path.is_file().then_some(path)
}

/// Program and args for the project terminal (spec §5):
/// PowerShell → pwsh.exe if found on PATH, else powershell.exe; args ["-NoLogo"].
/// GitBash → C:\Program Files\Git\bin\bash.exe; args ["--login", "-i"].
/// Cmd → cmd.exe; no args.
pub fn shell_command(kind: ShellKind) -> (PathBuf, Vec<String>) {
    match kind {
        ShellKind::PowerShell => {
            let program =
                find_on_path("pwsh.exe").unwrap_or_else(|| PathBuf::from("powershell.exe"));
            (program, vec!["-NoLogo".to_string()])
        }
        ShellKind::GitBash => (
            PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
            vec!["--login".to_string(), "-i".to_string()],
        ),
        ShellKind::Cmd => (PathBuf::from("cmd.exe"), Vec::new()),
    }
}

/// Searches each PATH entry for `exe` (exact file name), first match wins.
pub fn find_on_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_claude_returns_existing_setting_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("claude.exe");
        std::fs::write(&file, b"x").expect("write temp file");
        let setting = file.to_string_lossy().into_owned();
        assert_eq!(find_claude(Some(&setting)), Some(file));
    }

    #[test]
    fn find_claude_with_missing_setting_does_not_panic() {
        let _ = find_claude(Some(r"C:\nope\nope.exe"));
    }

    #[test]
    fn find_on_path_finds_cmd() {
        assert!(find_on_path("cmd.exe").is_some());
    }

    #[test]
    fn cmd_shell_is_cmd_exe() {
        let (program, args) = shell_command(ShellKind::Cmd);
        assert!(program.ends_with("cmd.exe"), "got {program:?}");
        assert!(args.is_empty());
    }

    #[test]
    fn git_bash_uses_login_interactive_args() {
        let (_, args) = shell_command(ShellKind::GitBash);
        assert_eq!(args, ["--login", "-i"]);
    }

    #[test]
    fn state_path_joins_state_json() {
        assert_eq!(
            state_path(Path::new(r"C:\x")),
            Path::new(r"C:\x\state.json")
        );
    }
}
