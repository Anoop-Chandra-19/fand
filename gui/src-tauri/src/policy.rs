//! Tauri commands for reassigning which curve(s) drive a channel — see
//! `fand_core::policy_edit` for the actual `toml_edit` surgery. Every
//! channel keeps its policy shape; these only change curve references.

use fand_proto::client::Client;

use crate::curves::{apply, CurveEditorPayload};
use crate::socket_path;

#[tauri::command]
pub fn set_channel_curve(
    channel: String,
    sensor: String,
    curve: String,
) -> Result<CurveEditorPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let current = client.get_config().map_err(|e| e.to_string())?;
    let updated = fand_core::set_channel_curve(&current, &channel, &sensor, &curve)
        .map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

#[tauri::command]
pub fn add_mix_input(
    channel: String,
    sensor: String,
    curve: String,
) -> Result<CurveEditorPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let current = client.get_config().map_err(|e| e.to_string())?;
    let updated = fand_core::add_mix_input(&current, &channel, &sensor, &curve)
        .map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

#[tauri::command]
pub fn remove_mix_input(channel: String, sensor: String) -> Result<CurveEditorPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let current = client.get_config().map_err(|e| e.to_string())?;
    let updated =
        fand_core::remove_mix_input(&current, &channel, &sensor).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}
