//! hooks.json text and file, and run_hook (the deck-host hook subcommand).

use crate::framed::write_frame;
use crate::pipe::{connect_trusted, current_user_sid};
use deck_proto::messages::{encode_request, ClientKind, Request};
use deck_proto::HOOK_ACK;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;

const WAIT: Duration = Duration::from_secs(2);

/// hooks.json for `--settings`: SessionStart, UserPromptSubmit, PreToolUse and PostToolUse
/// (matcher "*"), Notification, Stop, SessionEnd — each `{"type":"command","command":"'<exe>' hook"}`.
/// The exe path uses forward slashes inside single quotes (Claude runs hooks through a POSIX
/// shell, which would strip backslashes and expand `$` and backticks; a `'` becomes `'\''`).
/// Never sets "async", so events stay ordered.
pub fn hooks_json(host_exe: &Path) -> String {
    let exe = host_exe.to_string_lossy().replace('\x5c', "/");
    let command = format!("'{}' hook", exe.replace('\'', r"'\''"));
    let group = |matcher: Option<&str>| {
        let mut g = json!({ "hooks": [{ "type": "command", "command": command }] });
        if let Some(m) = matcher {
            g["matcher"] = json!(m);
        }
        json!([g])
    };
    let v = json!({
        "hooks": {
            "SessionStart": group(None),
            "UserPromptSubmit": group(None),
            "PreToolUse": group(Some("*")),
            "PostToolUse": group(Some("*")),
            "Notification": group(None),
            "Stop": group(None),
            "SessionEnd": group(None),
        }
    });
    serde_json::to_string_pretty(&v).expect("json serialises")
}

/// Writes hooks_json atomically (temp file + rename), creating the parent folder.
pub fn write_hooks_file(path: &Path, host_exe: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp_name);
    let result =
        std::fs::write(&tmp, hooks_json(host_exe)).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The `deck-host hook` subcommand. If `deck_id` is None or empty: return at once.
/// Otherwise: connect (wait up to 2 s), send Hello{Hook} then HookEvent{deck_id, payload: stdin},
/// then wait up to 2 s for one byte (HOOK_ACK). Every failure is silent.
pub async fn run_hook(stdin: String, deck_id: Option<String>, pipe_name: &str) {
    let Some(deck_id) = deck_id.filter(|d| !d.is_empty()) else {
        return;
    };
    let Ok(sid) = current_user_sid() else {
        return;
    };
    let Ok(Ok(mut pipe)) = tokio::time::timeout(WAIT, connect_trusted(pipe_name, WAIT, &sid)).await
    else {
        return;
    };
    let exchange = async {
        write_frame(
            &mut pipe,
            &encode_request(&Request::Hello {
                client: ClientKind::Hook,
            }),
        )
        .await?;
        write_frame(
            &mut pipe,
            &encode_request(&Request::HookEvent {
                deck_id,
                payload: stdin,
            }),
        )
        .await?;
        let mut b = [0u8; 1];
        let n = pipe.read(&mut b).await?;
        Ok::<bool, std::io::Error>(n == 1 && b[0] == HOOK_ACK)
    };
    let _ = tokio::time::timeout(WAIT, exchange).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framed::read_frame;
    use crate::pipe::{create_server, current_user_sid, PipeSecurity};
    use deck_proto::messages::{decode_request, ClientKind, Request};
    use deck_proto::{Frame, FrameDecoder, HOOK_ACK};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const EXE: &str = r"C:\Program Files\Claude Deck\deck-host.exe";

    fn unique_name() -> String {
        static N: AtomicU32 = AtomicU32::new(0);
        format!(
            r"\\.\pipe\deck-hook-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    }

    fn req(f: Frame) -> Request {
        match f {
            Frame::Control(b) => decode_request(&b).unwrap(),
            other => panic!("unexpected frame {other:?}"),
        }
    }

    #[test]
    fn hooks_json_shape() {
        let v: serde_json::Value = serde_json::from_str(&hooks_json(Path::new(EXE))).unwrap();
        let hooks = v["hooks"].as_object().unwrap();
        let mut keys: Vec<&str> = hooks.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut want = vec![
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Notification",
            "Stop",
            "SessionEnd",
        ];
        want.sort_unstable();
        assert_eq!(keys, want);
        for (name, groups) in hooks {
            let groups = groups.as_array().unwrap();
            assert_eq!(groups.len(), 1);
            let g = &groups[0];
            if name == "PreToolUse" || name == "PostToolUse" {
                assert_eq!(g["matcher"], "*");
            }
            let inner = g["hooks"].as_array().unwrap();
            assert_eq!(inner.len(), 1);
            assert_eq!(inner[0]["type"], "command");
            assert_eq!(
                inner[0]["command"],
                "'C:/Program Files/Claude Deck/deck-host.exe' hook"
            );
        }
    }

    #[test]
    fn hooks_json_quotes_dollar_space_and_apostrophe() {
        let v: serde_json::Value =
            serde_json::from_str(&hooks_json(Path::new(r"C:\a b\$HOME\it's`x`.exe"))).unwrap();
        assert_eq!(
            v["hooks"]["Stop"][0]["hooks"][0]["command"],
            r"'C:/a b/$HOME/it'\''s`x`.exe' hook"
        );
    }

    #[test]
    fn git_bash_runs_quoted_command() {
        let v: serde_json::Value =
            serde_json::from_str(&hooks_json(Path::new(r"C:\Windows\System32\whoami.exe")))
                .unwrap();
        let cmd = v["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        let bash = r"C:\Program Files\Git\bin\bash.exe";
        let out = std::process::Command::new(bash)
            .args(["-c", cmd])
            .output()
            .unwrap();
        let code = out.status.code().unwrap();
        assert!(code != 127 && code != 2, "bash exit {code}: {out:?}");
    }

    #[test]
    fn hooks_json_has_no_async() {
        assert!(!hooks_json(Path::new(EXE)).contains("async"));
    }

    #[test]
    fn write_hooks_file_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("hooks.json");
        write_hooks_file(&path, Path::new(EXE)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            hooks_json(Path::new(EXE))
        );
        let n = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(n, 1, "no temp file left behind");
        // overwrite works
        write_hooks_file(&path, Path::new(EXE)).unwrap();
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[tokio::test]
    async fn no_deck_id_returns_at_once() {
        let t = Instant::now();
        run_hook("{}".into(), None, r"\\.\pipe\deck-hook-test-nonexistent").await;
        run_hook(
            "{}".into(),
            Some(String::new()),
            r"\\.\pipe\deck-hook-test-nonexistent",
        )
        .await;
        assert!(t.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn forwards_and_waits_for_ack() {
        let name = unique_name();
        let sec = PipeSecurity::only(&current_user_sid().unwrap()).unwrap();
        let mut server = create_server(&name, &sec, true).unwrap();
        let srv = tokio::spawn(async move {
            server.connect().await.unwrap();
            let mut dec = FrameDecoder::new();
            let hello = req(read_frame(&mut server, &mut dec).await.unwrap().unwrap());
            let ev = req(read_frame(&mut server, &mut dec).await.unwrap().unwrap());
            server.write_all(&[HOOK_ACK]).await.unwrap();
            server.flush().await.unwrap();
            // keep open until the client closes
            let mut b = [0u8; 1];
            let _ = server.read(&mut b).await;
            (hello, ev)
        });
        let t = Instant::now();
        run_hook(r#"{"a":1}"#.into(), Some("deck-1".into()), &name).await;
        assert!(
            t.elapsed() < Duration::from_millis(1500),
            "took {:?}",
            t.elapsed()
        );
        let (hello, ev) = srv.await.unwrap();
        assert_eq!(
            hello,
            Request::Hello {
                client: ClientKind::Hook
            }
        );
        assert_eq!(
            ev,
            Request::HookEvent {
                deck_id: "deck-1".into(),
                payload: r#"{"a":1}"#.into()
            }
        );
    }

    #[tokio::test]
    async fn no_server_returns_soon() {
        let t = Instant::now();
        run_hook("{}".into(), Some("d".into()), &unique_name()).await;
        assert!(
            t.elapsed() < Duration::from_millis(2500),
            "took {:?}",
            t.elapsed()
        );
    }

    #[tokio::test]
    async fn server_never_acks_times_out() {
        let name = unique_name();
        let sec = PipeSecurity::only(&current_user_sid().unwrap()).unwrap();
        let server = create_server(&name, &sec, true).unwrap();
        let srv = tokio::spawn(async move {
            server.connect().await.unwrap();
            tokio::time::sleep(Duration::from_secs(4)).await;
            drop(server);
        });
        let t = Instant::now();
        run_hook("{}".into(), Some("d".into()), &name).await;
        assert!(
            t.elapsed() < Duration::from_millis(2500),
            "took {:?}",
            t.elapsed()
        );
        srv.abort();
    }
}
