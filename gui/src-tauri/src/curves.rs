//! Curve-editor Tauri commands: fetch the daemon's current curves (and
//! which channel binds each), and edit curves — graph points/sensor, mix
//! membership, channel bindings.

use std::collections::BTreeMap;

use fand_core::config::CurveConfig;
use fand_proto::client::Client;
use fand_proto::Command;
use serde::Serialize;

use crate::socket_path;

/// Mirrors `fand_core::config::CurveConfig` for the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CurveInfo {
    Graph {
        sensor: String,
        points: Vec<(i32, u16)>,
        hysteresis_up: f64,
        hysteresis_down: f64,
        response_seconds: u64,
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
    /// The daemon config generation this payload was built from; the
    /// frontend compares it against status frames to spot staleness.
    pub config_generation: u64,
}

pub(crate) fn payload_from_config(cfg: &fand_core::Config, generation: u64) -> CurveEditorPayload {
    let curves = cfg
        .curves
        .iter()
        .map(|(name, curve)| {
            let info = match curve {
                CurveConfig::Graph(g) => CurveInfo::Graph {
                    sensor: g.sensor.clone(),
                    points: g.points.clone(),
                    hysteresis_up: g.hysteresis_up,
                    hysteresis_down: g.hysteresis_down,
                    response_seconds: g.response_seconds,
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
        config_generation: generation,
    }
}

/// Validates an edited config, sends it to the daemon, then reads the
/// applied config back — the daemon owns the generation number, and the
/// re-read confirms what actually got applied. Shared by every write
/// command here and in `settings.rs`.
pub(crate) fn apply_config(
    updated: String,
    client: &mut Client,
) -> Result<(fand_core::Config, u64), String> {
    fand_core::Config::from_toml_str(&updated).map_err(|e| e.to_string())?;
    client
        .request(Command::SetConfig { toml: updated })
        .map_err(|e| e.to_string())?;
    let (applied, generation) = client.get_config().map_err(|e| e.to_string())?;
    let cfg = fand_core::Config::from_toml_str(&applied).map_err(|e| e.to_string())?;
    Ok((cfg, generation))
}

fn apply(updated: String, client: &mut Client) -> Result<CurveEditorPayload, String> {
    apply_config(updated, client).map(|(cfg, generation)| payload_from_config(&cfg, generation))
}

fn connect() -> Result<(Client, String), String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let (current, _) = client.get_config().map_err(|e| e.to_string())?;
    Ok((client, current))
}

#[tauri::command]
pub fn get_curve_editor_data() -> Result<CurveEditorPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let (toml_text, generation) = client.get_config().map_err(|e| e.to_string())?;
    let cfg = fand_core::Config::from_toml_str(&toml_text).map_err(|e| e.to_string())?;
    Ok(payload_from_config(&cfg, generation))
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

/// Applies a graph-curve edit as one batch — sensor, points, hysteresis and
/// response dwell in a single daemon round trip, so the editor's Apply can
/// never leave a half-edited curve on the hardware.
#[tauri::command]
pub fn apply_graph_curve(
    name: String,
    sensor: String,
    points: Vec<(i32, u8)>,
    hysteresis_up: f64,
    hysteresis_down: f64,
    response_seconds: u64,
) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated = fand_core::update_graph_curve(
        &current,
        &name,
        &sensor,
        &points,
        hysteresis_up,
        hysteresis_down,
        response_seconds,
    )
    .map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Creates a new flat curve holding a constant pwm.
#[tauri::command]
pub fn create_flat_curve(name: String, pwm: u8) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated = fand_core::create_flat_curve(&current, &name, pwm).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Changes an existing flat curve's constant pwm.
#[tauri::command]
pub fn set_flat_pwm(name: String, pwm: u8) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated = fand_core::set_flat_pwm(&current, &name, pwm).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Creates a new mix curve combining `members` with `function`.
#[tauri::command]
pub fn create_mix_curve(
    name: String,
    function: String,
    members: Vec<String>,
) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated = fand_core::create_mix_curve(&current, &name, &function, &members)
        .map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Changes an existing mix curve's combining function. The daemon-side
/// validation re-checks the safety rule that `min`/`average` are explicit
/// opt-ins; clients must keep displaying the chosen function.
#[tauri::command]
pub fn set_mix_function(name: String, function: String) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated =
        fand_core::set_mix_function(&current, &name, &function).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Creates a new trigger curve (validation enforces the pwm1 ban and the
/// deadband ordering).
#[tauri::command]
pub fn create_trigger_curve(
    name: String,
    sensor: String,
    idle_temp: f64,
    idle_pwm: u8,
    load_temp: f64,
    load_pwm: u8,
    response_seconds: u64,
) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated = fand_core::create_trigger_curve(
        &current,
        &name,
        &sensor,
        idle_temp,
        idle_pwm,
        load_temp,
        load_pwm,
        response_seconds,
    )
    .map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Applies a trigger-curve edit as one batch, mirroring `apply_graph_curve`.
#[tauri::command]
pub fn apply_trigger_curve(
    name: String,
    sensor: String,
    idle_temp: f64,
    idle_pwm: u8,
    load_temp: f64,
    load_pwm: u8,
    response_seconds: u64,
) -> Result<CurveEditorPayload, String> {
    let (mut client, current) = connect()?;
    let updated = fand_core::update_trigger_curve(
        &current,
        &name,
        &sensor,
        idle_temp,
        idle_pwm,
        load_temp,
        load_pwm,
        response_seconds,
    )
    .map_err(|e| e.to_string())?;
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
