//! Tauri backend: bridges the fand Unix socket to the webview.
//!
//! One background thread owns the socket. It subscribes to the daemon's
//! status stream and re-emits every frame as a `status` event carrying the
//! current config payload (see `state.rs` — the backend owns the only
//! config copy; the React side is a pure presentation layer). When the
//! daemon goes away it emits `daemon-down` on every retry — repeating it
//! means a webview that started listening late still learns the state —
//! and quietly retries until the stream is back.

mod curves;
mod settings;
mod state;

use std::path::PathBuf;
use std::time::Duration;

use fand_proto::client::Client;
use tauri::{AppHandle, Emitter, Manager};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// Floor for the status-stream read deadline. Frames arrive every daemon
/// tick, so a much longer silence means the daemon is wedged — treat it
/// as a disconnect instead of blocking on the socket forever. The tick
/// interval is user-configurable with no upper bound, so once the pump
/// knows it the deadline becomes `max(floor, 3 × tick)` — a fixed value
/// would declare a healthy slow-ticking daemon dead.
const STREAM_TIMEOUT_FLOOR: Duration = Duration::from_secs(15);

fn stream_timeout(tick_seconds: Option<u64>) -> Duration {
    match tick_seconds {
        Some(tick) => STREAM_TIMEOUT_FLOOR.max(Duration::from_secs(tick.saturating_mul(3))),
        None => STREAM_TIMEOUT_FLOOR,
    }
}

pub(crate) fn socket_path() -> PathBuf {
    std::env::var_os("FAND_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fand_proto::SOCKET_PATH))
}

fn status_pump(app: AppHandle) {
    let socket = socket_path();
    loop {
        // Each connection is a new daemon lifetime as far as generation
        // numbers go — drop anything cached under the previous counter.
        state::clear(&app);
        let stream = Client::connect_with_timeout(&socket, stream_timeout(None))
            .map_err(drop)
            .and_then(|client| client.subscribe().map_err(drop));
        // Set when a mid-frame config fetch answered from a different
        // daemon than the stream: the daemon restarted and a *new* one is
        // already up, so reconnect straight away — no daemon-down (untrue)
        // and no retry delay (nothing to wait for).
        let mut stale_stream = false;
        if let Ok(mut stream) = stream {
            let mut deadline = stream_timeout(None);
            while let Some(frame) = stream.next() {
                match frame {
                    Ok(status) => match state::emit_frame(&app, status) {
                        state::FrameOutcome::Emitted { tick_seconds } => {
                            // The covering config tells us the daemon's
                            // actual tick interval; keep the wedge-
                            // detection deadline proportional to it.
                            let wanted = stream_timeout(tick_seconds);
                            if wanted != deadline && stream.set_read_timeout(wanted).is_ok() {
                                deadline = wanted;
                            }
                        }
                        state::FrameOutcome::StaleStream => {
                            stale_stream = true;
                            break;
                        }
                    },
                    // Disconnect, timeout or garbage — either way the
                    // connection is unusable; fall through to reconnect.
                    Err(_) => break,
                }
            }
        }
        if stale_stream {
            // The webview must still learn the daemon changed: open
            // editing dialogs hold drafts from the old daemon's config,
            // and the disconnect path that normally closes them
            // (`daemon-down` → connected === false) is skipped here.
            let _ = app.emit("daemon-restarted", ());
        } else {
            let _ = app.emit("daemon-down", ());
            std::thread::sleep(RECONNECT_DELAY);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // App-defined commands aren't gated by capabilities/default.json's ACL
    // (that only covers plugin permissions like core:*) — verified with a
    // throwaway ping() command before wiring the real ones in.
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            curves::set_curve_points,
            curves::create_graph_curve,
            curves::apply_graph_curve,
            curves::create_flat_curve,
            curves::set_flat_pwm,
            curves::create_mix_curve,
            curves::set_mix_function,
            curves::create_trigger_curve,
            curves::apply_trigger_curve,
            curves::set_graph_sensor,
            curves::add_mix_member,
            curves::remove_mix_member,
            curves::set_channel_curve,
            curves::delete_curve,
            settings::set_min_pwm,
            settings::set_smoothing_seconds,
            settings::set_offset_pwm,
            settings::clear_override,
            settings::daemon_socket,
            settings::reload_config
        ])
        .setup(|app| {
            app.manage(state::SharedConfig::default());
            let handle = app.handle().clone();
            std::thread::spawn(move || status_pump(handle));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
