mod bridge;
mod usage;

use bridge::{Bridge, OutputChunk};
use deck_proto::messages::{encode_request, Request};
use deck_proto::Frame;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State, UserAttentionType, Window, WindowEvent};

#[tauri::command]
async fn deck_send(json: String, bridge: State<'_, Bridge>) -> Result<(), String> {
    serde_json::from_str::<Request>(&json).map_err(|e| format!("invalid request: {e}"))?;
    bridge.send(Frame::Control(json.into_bytes()))
}

#[tauri::command]
async fn deck_input(index: u32, data: Vec<u8>, bridge: State<'_, Bridge>) -> Result<(), String> {
    bridge.send(Frame::Input {
        session: index,
        data,
    })
}

#[tauri::command]
async fn deck_subscribe(
    output: Channel<OutputChunk>,
    bridge: State<'_, Bridge>,
) -> Result<(), String> {
    bridge
        .hub
        .subscribe(Box::new(move |chunk| output.send(chunk).is_ok()));
    Ok(())
}

#[tauri::command]
async fn deck_close(
    keep_running: bool,
    app: AppHandle,
    bridge: State<'_, Bridge>,
) -> Result<(), String> {
    bridge.closing.store(true, Ordering::SeqCst);
    bridge
        .send_flushed(
            encode_request(&Request::WindowClosing { keep_running }),
            Duration::from_secs(1),
        )
        .await;
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn deck_attention(title: String, flash: bool, window: Window) {
    let _ = window.set_title(&title);
    if flash && !window.is_focused().unwrap_or(true) {
        let _ = window.request_user_attention(Some(UserAttentionType::Informational));
    }
}

/// Claude plan usage for the footer (see `usage`).
#[tauri::command]
async fn claude_usage() -> usage::UsageReport {
    usage::fetch().await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let (b, rx) = Bridge::new();
            app.manage(b);
            tauri::async_runtime::spawn(bridge::run(app.handle().clone(), rx));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let closing = window
                    .try_state::<Bridge>()
                    .is_some_and(|b| b.closing.load(Ordering::SeqCst));
                if !closing {
                    api.prevent_close();
                    let _ = window.emit("deck:close-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            deck_send,
            deck_input,
            deck_subscribe,
            deck_close,
            deck_attention,
            claude_usage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
