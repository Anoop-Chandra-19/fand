//! Curve-editor Tauri commands: fetch the daemon's current curves (and
//! which channel binds each), and edit curves — graph points/sensor, mix
//! membership, channel bindings.

use std::collections::BTreeMap;

use fand_core::config::CurveConfig;
use fand_proto::client::Client;
use fand_proto::Command;
use serde::Serialize;

use crate::socket_path;

/// Mirrors `fand_core::config::CurveConfig` for the frontend, minus the
/// graph hysteresis fields (edited in the curve editor, phase 10). Trigger
/// curves are surfaced read-only until the phase-10 editor gains controls.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CurveInfo {
    Graph {
        sensor: String,
        points: Vec<(i32, u16)>,
    },
    Mix {
        function: String,
        members: Vec<String>,
    },
    Flat {
        pwm: u16,
    },
    Trigger {
        sensor: String,
        idle_temp: f64,
        idle_pwm: u16,
        load_temp: f64,
        load_pwm: u16,
        response_seconds: u64,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct CurveEditorPayload {
    pub curves: BTreeMap<String, CurveInfo>,
    /// channel name → the curve it binds.
    pub channels: BTreeMap<String, String>,
    /// Already-configured sensor names, for graph-curve sensor pickers.
    pub sensors: Vec<String>,
}

pub(crate) fn payload_from_config(cfg: &fand_core::Config) -> CurveEditorPayload {
    let curves = cfg
        .curves
        .iter()
        .map(|(name, curve)| {
            let info = match curve {
                CurveConfig::Graph(g) => CurveInfo::Graph {
                    sensor: g.sensor.clone(),
                    points: g.points.clone(),
                },
                CurveConfig::Mix(m) => CurveInfo::Mix {
                    function: m.function.as_str().to_string(),
                    members: m.curves.clone(),
                },
                CurveConfig::Flat(f) => CurveInfo::Flat { pwm: f.pwm },
                CurveConfig::Trigger(t) => CurveInfo::Trigger {
                    sensor: t.sensor.clone(),
                    idle_temp: t.idle_temp,
                    idle_pwm: t.idle_pwm,
                    load_temp: t.load_temp,
                    load_pwm: t.load_pwm,
                    response_seconds: t.response_seconds,
                },
            };
            (name.clone(), info)
        })
        .collect();

    let channels = cfg
        .channels
        .iter()
        .map(|(name, channel)| (name.clone(), channel.curve.clone()))
        .collect();

    let sensors = cfg.sensors.keys().cloned().collect();

    CurveEditorPayload {
        curves,
        channels,
        sensors,
    }
}

/// Validates an edited config and sends it to the daemon, returning the
/// fresh daemon-confirmed `Config` — shared by every write command here and
/// in `settings.rs` so none of them repeat the validate/send sequence.
pub(crate) fn apply_config(
    updated: String,
    client: &mut Client,
) -> Result<fand_core::Config, String> {
    let cfg = fand_core::Config::from_toml_str(&updated).map_err(|e| e.to_string())?;
    client
        .request(Command::SetConfig { toml: updated })
        .map_err(|e| e.to_string())?;
    Ok(cfg)
}

fn apply(updated: String, client: &mut Client) -> Result<CurveEditorPayload, String> {
    apply_config(updated, client).map(|cfg| payload_from_config(&cfg))
}

fn connect() -> Result<(Client, String), String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let current = client.get_config().map_err(|e| e.to_string())?;
    Ok((client, current))
}

#[tauri::command]
pub fn get_curve_editor_data() -> Result<CurveEditorPayload, String> {
    let (_, toml_text) = connect()?;
    let cfg = fand_core::Config::from_toml_str(&toml_text).map_err(|e| e.to_string())?;
    Ok(payload_from_config(&cfg))
}

/// Replaces an existing graph curve's points. Returns the fresh,
/// daemon-confirmed payload so the frontend never has to guess what
/// actually got applied.
#[tauri::command]
pub fn set_curve_points(
    name: String,
    points: Vec<(i32, u8)>,
) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated =
        fand_core::replace_curve_points(&current, &name, &points).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Creates a new graph curve bound to `sensor`.
#[tauri::command]
pub fn create_graph_curve(
    name: String,
    sensor: String,
    points: Vec<(i32, u8)>,
) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated = fand_core::create_graph_curve(&current, &name, &sensor, &points)
        .map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Rebinds which sensor drives a graph curve.
#[tauri::command]
pub fn set_graph_sensor(name: String, sensor: String) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated =
        fand_core::set_graph_sensor(&current, &name, &sensor).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Adds a member curve to a mix.
#[tauri::command]
pub fn add_mix_member(name: String, member: String) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated = fand_core::add_mix_member(&current, &name, &member).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Removes a member curve from a mix (the daemon rejects dropping to zero).
#[tauri::command]
pub fn remove_mix_member(name: String, member: String) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated =
        fand_core::remove_mix_member(&current, &name, &member).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Rebinds which curve drives a channel.
#[tauri::command]
pub fn set_channel_curve(channel: String, curve: String) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated =
        fand_core::set_channel_curve(&current, &channel, &curve).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Removes a curve. Fails (daemon-validated) if any channel or mix still
/// references it — the frontend should already be disabling this for
/// in-use curves, but the daemon is the backstop.
#[tauri::command]
pub fn delete_curve(name: String) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated = fand_core::remove_curve(&current, &name).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}
