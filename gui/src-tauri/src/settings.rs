//! Tauri commands for the channel-properties dialog — min_pwm, smoothing
//! and offset tuning, plus clearing a manual override. See
//! `fand_core::channel_edit` for the actual `toml_edit` surgery.

use std::collections::BTreeMap;

use fand_proto::client::Client;
use fand_proto::Command;
use serde::Serialize;

use crate::curves::apply_config;
use crate::socket_path;

#[derive(Debug, Clone, Serialize)]
pub struct ChannelSettings {
    pub min_pwm: u8,
    pub smoothing_seconds: u64,
    pub offset_pwm: i16,
}

/// Carries the daemon config generation it was built from, like the curve
/// payload — the frontend compares it against status frames to spot a
/// stale copy even when only this payload's fetch failed.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelSettingsPayload {
    pub channels: BTreeMap<String, ChannelSettings>,
    pub config_generation: u64,
}

fn payload_from_config(cfg: &fand_core::Config, generation: u64) -> ChannelSettingsPayload {
    ChannelSettingsPayload {
        channels: cfg
            .channels
            .iter()
            .map(|(name, channel)| {
                (
                    name.clone(),
                    ChannelSettings {
                        min_pwm: channel.min_pwm,
                        smoothing_seconds: channel.smoothing_seconds,
                        offset_pwm: channel.offset_pwm,
                    },
                )
            })
            .collect(),
        config_generation: generation,
    }
}

#[tauri::command]
pub fn get_channel_settings() -> Result<ChannelSettingsPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let (toml_text, generation) = client.get_config().map_err(|e| e.to_string())?;
    let cfg = fand_core::Config::from_toml_str(&toml_text).map_err(|e| e.to_string())?;
    Ok(payload_from_config(&cfg, generation))
}

#[tauri::command]
pub fn set_min_pwm(channel: String, min_pwm: u8) -> Result<ChannelSettingsPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let (current, _) = client.get_config().map_err(|e| e.to_string())?;
    let updated = fand_core::set_min_pwm(&current, &channel, min_pwm).map_err(|e| e.to_string())?;
    apply_config(updated, &mut client)
        .map(|(cfg, generation)| payload_from_config(&cfg, generation))
}

#[tauri::command]
pub fn set_smoothing_seconds(
    channel: String,
    seconds: u64,
) -> Result<ChannelSettingsPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let (current, _) = client.get_config().map_err(|e| e.to_string())?;
    let updated =
        fand_core::set_smoothing_seconds(&current, &channel, seconds).map_err(|e| e.to_string())?;
    apply_config(updated, &mut client)
        .map(|(cfg, generation)| payload_from_config(&cfg, generation))
}

#[tauri::command]
pub fn set_offset_pwm(channel: String, offset: i16) -> Result<ChannelSettingsPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let (current, _) = client.get_config().map_err(|e| e.to_string())?;
    let updated =
        fand_core::set_offset_pwm(&current, &channel, offset).map_err(|e| e.to_string())?;
    apply_config(updated, &mut client)
        .map(|(cfg, generation)| payload_from_config(&cfg, generation))
}

/// Cancels a manual override, handing the channel back to its curve. The
/// next status frame reflects the change, so there is no payload to return.
#[tauri::command]
pub fn clear_override(channel: String) -> Result<(), String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    client
        .request(Command::ClearOverride { channel })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Where this window talks to the daemon — shown in Preferences.
#[tauri::command]
pub fn daemon_socket() -> String {
    socket_path().display().to_string()
}

/// Asks the daemon to re-read its config file from disk and hot-apply it
/// (same validation as any other config change).
#[tauri::command]
pub fn reload_config() -> Result<(), String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    client
        .request(Command::ReloadConfig)
        .map_err(|e| e.to_string())?;
    Ok(())
}
