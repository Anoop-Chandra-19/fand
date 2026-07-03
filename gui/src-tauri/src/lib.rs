//! Tauri backend: bridges the fand Unix socket to the webview.
//!
//! One background thread owns the socket. It subscribes to the daemon's
//! status stream and re-emits every frame as a `status` event; the React
//! side never sees the wire protocol. When the daemon goes away (restart,
//! failsafe exit, not running yet) it emits `daemon-down` once and quietly
//! retries until the stream is back.

mod curves;
mod policy;

use std::path::PathBuf;
use std::time::Duration;

use fand_proto::client::Client;
use tauri::{AppHandle, Emitter};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub(crate) fn socket_path() -> PathBuf {
    std::env::var_os("FAND_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fand_proto::SOCKET_PATH))
}

fn status_pump(app: AppHandle) {
    let socket = socket_path();
    let mut announced_down = false;
    loop {
        let stream = Client::connect(&socket)
            .map_err(drop)
            .and_then(|client| client.subscribe().map_err(drop));
        if let Ok(stream) = stream {
            for frame in stream {
                match frame {
                    Ok(status) => {
                        announced_down = false;
                        let _ = app.emit("status", &status);
                    }
                    // Disconnect or garbage — either way the connection
                    // is unusable; fall through to reconnect.
                    Err(_) => break,
                }
            }
        }
        if !announced_down {
            let _ = app.emit("daemon-down", ());
            announced_down = true;
        }
        std::thread::sleep(RECONNECT_DELAY);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // App-defined commands aren't gated by capabilities/default.json's ACL
    // (that only covers plugin permissions like core:*) — verified with a
    // throwaway ping() command before wiring the real ones in.
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            curves::get_curve_editor_data,
            curves::set_curve_points,
            curves::delete_curve,
            policy::set_channel_curve,
            policy::add_mix_input,
            policy::remove_mix_input
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || status_pump(handle));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
