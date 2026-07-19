//! Tauri commands for the channel-properties dialog — min_pwm, smoothing
//! and offset tuning, plus clearing a manual override. See
//! `fand_core::channel_edit` for the actual `toml_edit` surgery. Like the
//! curve commands, writes return nothing on success (the applied config
//! reaches the webview as a `config` event via the shared cache), run
//! their blocking socket I/O in `spawn_blocking`, and — for everything
//! that mutates config — go through the one write gate (see `curves.rs`).
//! `clear_override` is a direct command, not a config read-modify-write,
//! so it skips the gate; `reload_config` mutates config and takes it.

use fand_proto::client::{Client, ClientError};
use fand_proto::Command;
use tauri::AppHandle;

use crate::curves::{run_write, write_gate, WriteResult};
use crate::socket_path;
use crate::state::REQUEST_TIMEOUT;

#[tauri::command]
pub async fn set_min_pwm(app: AppHandle, channel: String, min_pwm: u8) -> WriteResult {
    run_write(app, move |current| {
        fand_core::set_min_pwm(current, &channel, min_pwm).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn set_smoothing_seconds(app: AppHandle, channel: String, seconds: u64) -> WriteResult {
    run_write(app, move |current| {
        fand_core::set_smoothing_seconds(current, &channel, seconds).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn set_offset_pwm(app: AppHandle, channel: String, offset: i16) -> WriteResult {
    run_write(app, move |current| {
        fand_core::set_offset_pwm(current, &channel, offset).map_err(|e| e.to_string())
    })
    .await
}

/// Cancels a manual override, handing the channel back to its curve. The
/// next status frame reflects the change, so there is no config to publish
/// — and no config RMW, so no write gate.
#[tauri::command]
pub async fn clear_override(channel: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut client = Client::connect_with_timeout(socket_path(), REQUEST_TIMEOUT)
            .map_err(|e| e.to_string())?;
        match client.request_mutating(Command::ClearOverride { channel }) {
            Ok(_) => Ok(()),
            Err(ClientError::OutcomeUnknown(cause)) => Err(format!(
                "The daemon did not confirm clearing the override ({cause}) — it may or may \
                 not have applied; the next status frame will show the actual state."
            )),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| format!("internal: task failed: {e}"))?
}

/// Where this window talks to the daemon — shown in Preferences. No I/O,
/// so it may stay a sync (main-thread) command.
#[tauri::command]
pub fn daemon_socket() -> String {
    socket_path().display().to_string()
}

/// Asks the daemon to re-read its config file from disk and hot-apply it
/// (same validation as any other config change). The generation bump shows
/// up in the next status frame, which is what refreshes the webview.
/// Takes the write gate: a reload interleaving inside another command's
/// fetch→edit→apply span would be silently overwritten by it.
#[tauri::command]
pub async fn reload_config() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let _gate = write_gate();
        let mut client = Client::connect_with_timeout(socket_path(), REQUEST_TIMEOUT)
            .map_err(|e| e.to_string())?;
        match client.request_mutating(Command::ReloadConfig) {
            Ok(_) => Ok(()),
            Err(ClientError::OutcomeUnknown(cause)) => Err(format!(
                "The daemon did not confirm the reload ({cause}) — it may or may not have \
                 applied; the dashboard will show the actual state shortly."
            )),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| format!("internal: task failed: {e}"))?
}
