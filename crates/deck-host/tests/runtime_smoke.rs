//! Small smoke test of the runtime through a real pipe. Task 12 has the full end-to-end tests.

use std::sync::Arc;
use std::time::Duration;

use deck_core::store::SavedState;
use deck_host::framed::{read_frame, write_frame};
use deck_host::host::core::{Core, CoreConfig};
use deck_host::host::runtime::{run, system_clock, RuntimeConfig};
use deck_host::pipe::{connect_client, current_user_sid, PipeSecurity};
use deck_proto::messages::{decode_event, encode_request, ClientKind, Event, Request, StateView};
use deck_proto::{Frame, FrameDecoder};

async fn next_state<R: tokio::io::AsyncRead + Unpin>(
    r: &mut R,
    dec: &mut FrameDecoder,
) -> StateView {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Frame::Control(json) = read_frame(r, dec).await.unwrap().expect("pipe closed") {
                if let Event::State { state } = decode_event(&json).unwrap() {
                    return state;
                }
            }
        }
    })
    .await
    .expect("no State within 5 s")
}

#[tokio::test]
async fn hello_add_project_then_idle_exit() {
    let dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let name = format!(r"\\.\pipe\claude-deck-test-{}-1", std::process::id());
    let sid = current_user_sid().unwrap();
    let core = Core::new(
        SavedState::default(),
        CoreConfig {
            claude_exe: env!("CARGO_BIN_EXE_fake-claude").into(),
            hooks_file: dir.path().join("hooks.json"),
            shell: ("cmd.exe".into(), vec![]),
            ctrl_c_gap_ms: 300,
            sleep_grace_ms: 5000,
            idle_exit_ms: 1000,
            memory_every_ms: 5000,
            pipe_name: Some(name.clone()),
        },
    );
    let cfg = RuntimeConfig {
        pipe_name: name.clone(),
        security: Arc::new(PipeSecurity::only(&sid).unwrap()),
        state_path: dir.path().join("state.json"),
        claude_projects_dir: dir.path().join("projects"),
        tick_ms: 100,
        clock: system_clock(),
        max_window_queue_bytes: deck_host::host::runtime::DEFAULT_MAX_WINDOW_QUEUE,
    };
    let host = tokio::spawn(run(core, cfg));

    // run() creates the pipe on its own task; retry until it exists.
    let mut pipe = loop {
        match connect_client(&name, Duration::from_secs(3)).await {
            Ok(p) => break p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(e) => panic!("connect failed: {e}"),
        }
    };
    let mut dec = FrameDecoder::new();
    write_frame(
        &mut pipe,
        &encode_request(&Request::Hello {
            client: ClientKind::Window,
        }),
    )
    .await
    .unwrap();
    let first = next_state(&mut pipe, &mut dec).await;
    assert!(first.projects.is_empty());

    let path = project.path().to_string_lossy().to_string();
    write_frame(
        &mut pipe,
        &encode_request(&Request::AddProject { path: path.clone() }),
    )
    .await
    .unwrap();
    let after = next_state(&mut pipe, &mut dec).await;
    assert!(after.projects.iter().any(|p| p.path == path));

    drop(pipe);
    let result = tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .expect("run did not return within idle_exit_ms + 2 s")
        .unwrap();
    result.unwrap();
    assert!(
        dir.path().join("state.json").exists(),
        "pending save must be flushed on exit"
    );
}
