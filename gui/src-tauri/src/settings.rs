//! Tauri commands for the per-channel settings panel — min_pwm, smoothing,
//! and zero_rpm/kick tuning. See `fand_core::channel_edit` for the actual
//! `toml_edit` surgery.

use std::collections::BTreeMap;

use fand_proto::client::Client;
use serde::Serialize;

use crate::curves::apply_config;
use crate::socket_path;

#[derive(Debug, Clone, Serialize)]
pub struct ChannelSettings {
    pub min_pwm: u8,
    pub smoothing_seconds: u64,
    pub zero_rpm: bool,
    pub kick_pwm: Option<u8>,
    pub kick_seconds: Option<u64>,
}

fn payload_from_config(cfg: &fand_core::Config) -> BTreeMap<String, ChannelSettings> {
    cfg.channels
        .iter()
        .map(|(name, channel)| {
            (
                name.clone(),
                ChannelSettings {
                    min_pwm: channel.min_pwm,
                    smoothing_seconds: channel.smoothing_seconds,
                    zero_rpm: channel.zero_rpm,
                    kick_pwm: channel.kick_pwm,
                    kick_seconds: channel.kick_seconds,
                },
            )
        })
        .collect()
}

#[tauri::command]
pub fn get_channel_settings() -> Result<BTreeMap<String, ChannelSettings>, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let toml_text = client.get_config().map_err(|e| e.to_string())?;
    let cfg = fand_core::Config::from_toml_str(&toml_text).map_err(|e| e.to_string())?;
    Ok(payload_from_config(&cfg))
}

#[tauri::command]
pub fn set_min_pwm(channel: String, min_pwm: u8) -> Result<BTreeMap<String, ChannelSettings>, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let current = client.get_config().map_err(|e| e.to_string())?;
    let updated = fand_core::set_min_pwm(&current, &channel, min_pwm).map_err(|e| e.to_string())?;
    apply_config(updated, &mut client).map(|cfg| payload_from_config(&cfg))
}

#[tauri::command]
pub fn set_smoothing_seconds(
    channel: String,
    seconds: u64,
) -> Result<BTreeMap<String, ChannelSettings>, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let current = client.get_config().map_err(|e| e.to_string())?;
    let updated =
        fand_core::set_smoothing_seconds(&current, &channel, seconds).map_err(|e| e.to_string())?;
    apply_config(updated, &mut client).map(|cfg| payload_from_config(&cfg))
}

#[tauri::command]
pub fn set_zero_rpm(
    channel: String,
    zero_rpm: bool,
    kick_pwm: Option<u8>,
    kick_seconds: Option<u64>,
) -> Result<BTreeMap<String, ChannelSettings>, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let current = client.get_config().map_err(|e| e.to_string())?;
    let updated = fand_core::set_zero_rpm(&current, &channel, zero_rpm, kick_pwm, kick_seconds)
        .map_err(|e| e.to_string())?;
    apply_config(updated, &mut client).map(|cfg| payload_from_config(&cfg))
}
