//! Curve-editor Tauri commands: fetch the daemon's current curves (and
//! which channel(s) reference each), and edit a curve's points.

use std::collections::BTreeMap;

use fand_core::config::Policy;
use fand_proto::client::Client;
use fand_proto::Command;
use serde::Serialize;

use crate::socket_path;

#[derive(Debug, Clone, Serialize)]
pub struct CurveRef {
    pub sensor: String,
    pub curve: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelCurveRefs {
    /// One entry for a `single` policy, one per input for `mix`.
    pub refs: Vec<CurveRef>,
    /// Distinguishes the two shapes even when `refs.len() == 1` for both
    /// (a `mix` channel can have exactly one input, e.g. mid-edit).
    pub is_mix: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurveEditorPayload {
    pub curves: BTreeMap<String, Vec<(i32, u16)>>,
    pub channels: BTreeMap<String, ChannelCurveRefs>,
    /// Already-configured sensor names, for the "add mix input" picker —
    /// this UI reassigns curves on existing sensor bindings, it doesn't
    /// create new ones.
    pub sensors: Vec<String>,
}

pub(crate) fn payload_from_config(cfg: &fand_core::Config) -> CurveEditorPayload {
    let curves = cfg
        .curves
        .iter()
        .map(|(name, curve)| (name.clone(), curve.points.clone()))
        .collect();

    let channels = cfg
        .channels
        .iter()
        .map(|(name, channel)| {
            let (refs, is_mix) = match &channel.policy {
                Policy::Single { sensor, curve } => (
                    vec![CurveRef {
                        sensor: sensor.clone(),
                        curve: curve.clone(),
                    }],
                    false,
                ),
                Policy::Mix { inputs } => (
                    inputs
                        .iter()
                        .map(|input| CurveRef {
                            sensor: input.sensor.clone(),
                            curve: input.curve.clone(),
                        })
                        .collect(),
                    true,
                ),
            };
            (name.clone(), ChannelCurveRefs { refs, is_mix })
        })
        .collect();

    let sensors = cfg.sensors.keys().cloned().collect();

    CurveEditorPayload {
        curves,
        channels,
        sensors,
    }
}

/// Validates an edited config and sends it to the daemon, returning the
/// fresh daemon-confirmed `Config` — shared by every write command across
/// `curves.rs`, `policy.rs`, and `settings.rs` so none of them have to
/// repeat the validate/send sequence, even though each builds a different
/// payload shape from the result.
pub(crate) fn apply_config(updated: String, client: &mut Client) -> Result<fand_core::Config, String> {
    let cfg = fand_core::Config::from_toml_str(&updated).map_err(|e| e.to_string())?;
    client
        .request(Command::SetConfig { toml: updated })
        .map_err(|e| e.to_string())?;
    Ok(cfg)
}

pub(crate) fn apply(updated: String, client: &mut Client) -> Result<CurveEditorPayload, String> {
    apply_config(updated, client).map(|cfg| payload_from_config(&cfg))
}

#[tauri::command]
pub fn get_curve_editor_data() -> Result<CurveEditorPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let toml_text = client.get_config().map_err(|e| e.to_string())?;
    let cfg = fand_core::Config::from_toml_str(&toml_text).map_err(|e| e.to_string())?;
    Ok(payload_from_config(&cfg))
}

/// Edits an existing curve's points, or creates a new curve with these
/// points if `name` doesn't exist yet. Returns the fresh, daemon-confirmed
/// payload so the frontend never has to guess what actually got applied.
#[tauri::command]
pub fn set_curve_points(name: String, points: Vec<(i32, u8)>) -> Result<CurveEditorPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let current = client.get_config().map_err(|e| e.to_string())?;
    let updated =
        fand_core::replace_curve_points(&current, &name, &points).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}

/// Removes a curve. Fails (daemon-validated) if any channel still
/// references it — the frontend should already be disabling this for
/// in-use curves, but the daemon is the backstop.
#[tauri::command]
pub fn delete_curve(name: String) -> Result<CurveEditorPayload, String> {
    let mut client = Client::connect(socket_path()).map_err(|e| e.to_string())?;
    let current = client.get_config().map_err(|e| e.to_string())?;
    let updated = fand_core::remove_curve(&current, &name).map_err(|e| e.to_string())?;
    apply(updated, &mut client)
}
