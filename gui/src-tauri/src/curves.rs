//! Curve-editing Tauri commands — graph points/sensor, mix membership,
//! channel bindings. Commands return `Ok(None)` on plain success or
//! `Ok(Some(warning))` for applied-with-caveat outcomes (applied but not
//! persisted; applied to a restarted-away daemon) — the warning rides
//! the invoke result so the frontend produces exactly one toast per
//! operation, with no cross-channel ordering race. The applied config
//! itself reaches the webview as a `config` event published through the
//! shared cache (`state::publish`).
//!
//! Two rules protect these whole-config read-modify-writes:
//!
//! - **Every command is `async` and does its blocking work in
//!   `spawn_blocking`**: Tauri runs sync commands on the main thread, and
//!   these do blocking socket I/O — bounded by REQUEST_TIMEOUT, but a
//!   wedged daemon must cost a background thread a few seconds, never
//!   freeze the window or occupy async-runtime workers.
//! - **One write at a time** (`WRITE_GATE`): two concurrent RMWs could
//!   both read config generation N and the later SetConfig would erase
//!   the earlier edit. The gate covers the whole fetch→edit→apply span —
//!   including `reload_config` in `settings.rs`, which mutates without an
//!   RMW of its own but must not interleave inside someone else's. The
//!   daemon's compare-and-set (`expected`) backstops the gate against
//!   writers *outside* this process (fanctl).

use std::sync::{Mutex, MutexGuard};

use fand_proto::client::{Client, ClientError, ConfigSnapshot};
use fand_proto::SetConfigResult;
use tauri::AppHandle;

use crate::socket_path;
use crate::state::{self, PublishOutcome, REQUEST_TIMEOUT};

/// `Ok(None)` = clean success; `Ok(Some(warning))` = the mutation
/// applied, with a caveat the user must see.
pub(crate) type WriteResult = Result<Option<String>, String>;

static WRITE_GATE: Mutex<()> = Mutex::new(());

/// Take the process-wide write gate. Poisoning is recovered deliberately:
/// a panicked write held no partial state (the daemon owns the truth), so
/// the gate stays usable.
pub(crate) fn write_gate() -> MutexGuard<'static, ()> {
    WRITE_GATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn connect() -> Result<(Client, ConfigSnapshot), String> {
    let mut client =
        Client::connect_with_timeout(socket_path(), REQUEST_TIMEOUT).map_err(|e| e.to_string())?;
    let snap = client.get_config().map_err(|e| e.to_string())?;
    Ok((client, snap))
}

/// One complete, serialized read-modify-write: fetch the current config,
/// run `edit` on its TOML, compare-and-set the result back. Shared by
/// every write command here and in `settings.rs`.
pub(crate) async fn run_write<F>(app: AppHandle, edit: F) -> WriteResult
where
    F: FnOnce(&str) -> Result<String, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = write_gate();
        let (mut client, snap) = connect()?;
        let updated = edit(&snap.toml)?;
        // Instant local feedback; the daemon re-validates anyway.
        fand_core::Config::from_toml_str(&updated).map_err(|e| e.to_string())?;
        match client.set_config(updated, snap.version) {
            Ok(SetConfigResult::Applied { toml, version, .. }) => {
                Ok(publish_applied(&app, &toml, version, None))
            }
            Ok(SetConfigResult::AppliedButNotPersisted {
                toml,
                version,
                error,
            }) => Ok(publish_applied(&app, &toml, version, Some(error))),
            Ok(SetConfigResult::Conflict { .. }) => Err(
                // The gate serializes this process, so the concurrent
                // writer was external (fanctl) — retry-on-fresh-state is
                // the user's call, not ours.
                "The config changed while this edit was being applied — nothing was changed. \
                 Try again."
                    .to_string(),
            ),
            Ok(SetConfigResult::Rejected { error }) => Err(error),
            Err(ClientError::OutcomeUnknown(cause)) => Err(format!(
                "The daemon did not confirm this change ({cause}) — it may or may not have \
                 applied; the dashboard will show the actual state shortly."
            )),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| format!("internal: write task failed: {e}"))?
}

/// The mutation IS applied — whatever happens here only affects how the
/// webview learns about it, so this never fails; it returns the warning
/// (if any) for the command's single-toast result.
fn publish_applied(
    app: &AppHandle,
    toml: &str,
    version: fand_proto::ConfigVersion,
    mut warning: Option<String>,
) -> Option<String> {
    match fand_core::Config::from_toml_str(toml) {
        Ok(cfg) => {
            let outcome = state::publish(app, state::payload_from_config(&cfg, version));
            if outcome == PublishOutcome::StaleInstance {
                warning = Some(match warning {
                    Some(w) => format!(
                        "{w} — and the daemon restarted meanwhile; the dashboard shows the \
                         current daemon, which may differ"
                    ),
                    None => "Change applied to a daemon that has since restarted — the \
                             dashboard shows the current daemon, which may differ"
                        .to_string(),
                });
            }
        }
        Err(e) => {
            warning = Some(format!(
                "Change applied, but refreshing the view failed ({e}) — the dashboard will \
                 catch up shortly"
            ));
        }
    }
    warning
}

/// Replaces an existing graph curve's points.
#[tauri::command]
pub async fn set_curve_points(app: AppHandle, name: String, points: Vec<(i32, u8)>) -> WriteResult {
    run_write(app, move |current| {
        fand_core::replace_curve_points(current, &name, &points).map_err(|e| e.to_string())
    })
    .await
}

/// Creates a new graph curve bound to `sensor`.
#[tauri::command]
pub async fn create_graph_curve(
    app: AppHandle,
    name: String,
    sensor: String,
    points: Vec<(i32, u8)>,
) -> WriteResult {
    run_write(app, move |current| {
        fand_core::create_graph_curve(current, &name, &sensor, &points).map_err(|e| e.to_string())
    })
    .await
}

/// Rebinds which sensor drives a graph curve.
#[tauri::command]
pub async fn set_graph_sensor(app: AppHandle, name: String, sensor: String) -> WriteResult {
    run_write(app, move |current| {
        fand_core::set_graph_sensor(current, &name, &sensor).map_err(|e| e.to_string())
    })
    .await
}

/// Applies a graph-curve edit as one batch — sensor, points, hysteresis and
/// response dwell in a single daemon round trip, so the editor's Apply can
/// never leave a half-edited curve on the hardware.
#[tauri::command]
pub async fn apply_graph_curve(
    app: AppHandle,
    name: String,
    sensor: String,
    points: Vec<(i32, u8)>,
    hysteresis_up: f64,
    hysteresis_down: f64,
    response_seconds: u64,
) -> WriteResult {
    run_write(app, move |current| {
        fand_core::update_graph_curve(
            current,
            &name,
            &sensor,
            &points,
            hysteresis_up,
            hysteresis_down,
            response_seconds,
        )
        .map_err(|e| e.to_string())
    })
    .await
}

/// Creates a new flat curve holding a constant pwm.
#[tauri::command]
pub async fn create_flat_curve(app: AppHandle, name: String, pwm: u8) -> WriteResult {
    run_write(app, move |current| {
        fand_core::create_flat_curve(current, &name, pwm).map_err(|e| e.to_string())
    })
    .await
}

/// Changes an existing flat curve's constant pwm.
#[tauri::command]
pub async fn set_flat_pwm(app: AppHandle, name: String, pwm: u8) -> WriteResult {
    run_write(app, move |current| {
        fand_core::set_flat_pwm(current, &name, pwm).map_err(|e| e.to_string())
    })
    .await
}

/// Creates a new mix curve combining `members` with `function`.
#[tauri::command]
pub async fn create_mix_curve(
    app: AppHandle,
    name: String,
    function: String,
    members: Vec<String>,
) -> WriteResult {
    run_write(app, move |current| {
        fand_core::create_mix_curve(current, &name, &function, &members).map_err(|e| e.to_string())
    })
    .await
}

/// Changes an existing mix curve's combining function. The daemon-side
/// validation re-checks the safety rule that `min`/`average` are explicit
/// opt-ins; clients must keep displaying the chosen function.
#[tauri::command]
pub async fn set_mix_function(app: AppHandle, name: String, function: String) -> WriteResult {
    run_write(app, move |current| {
        fand_core::set_mix_function(current, &name, &function).map_err(|e| e.to_string())
    })
    .await
}

/// Creates a new trigger curve (validation enforces the pwm1 ban and the
/// deadband ordering). The arg list mirrors the trigger's wire fields —
/// bundling them in a struct would only move the count elsewhere.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_trigger_curve(
    app: AppHandle,
    name: String,
    sensor: String,
    idle_temp: f64,
    idle_pwm: u8,
    load_temp: f64,
    load_pwm: u8,
    response_seconds: u64,
) -> WriteResult {
    run_write(app, move |current| {
        fand_core::create_trigger_curve(
            current,
            &name,
            &sensor,
            idle_temp,
            idle_pwm,
            load_temp,
            load_pwm,
            response_seconds,
        )
        .map_err(|e| e.to_string())
    })
    .await
}

/// Applies a trigger-curve edit as one batch, mirroring `apply_graph_curve`.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn apply_trigger_curve(
    app: AppHandle,
    name: String,
    sensor: String,
    idle_temp: f64,
    idle_pwm: u8,
    load_temp: f64,
    load_pwm: u8,
    response_seconds: u64,
) -> WriteResult {
    run_write(app, move |current| {
        fand_core::update_trigger_curve(
            current,
            &name,
            &sensor,
            idle_temp,
            idle_pwm,
            load_temp,
            load_pwm,
            response_seconds,
        )
        .map_err(|e| e.to_string())
    })
    .await
}

/// Adds a member curve to a mix.
#[tauri::command]
pub async fn add_mix_member(app: AppHandle, name: String, member: String) -> WriteResult {
    run_write(app, move |current| {
        fand_core::add_mix_member(current, &name, &member).map_err(|e| e.to_string())
    })
    .await
}

/// Removes a member curve from a mix (the daemon rejects dropping to zero).
#[tauri::command]
pub async fn remove_mix_member(app: AppHandle, name: String, member: String) -> WriteResult {
    run_write(app, move |current| {
        fand_core::remove_mix_member(current, &name, &member).map_err(|e| e.to_string())
    })
    .await
}

/// Rebinds which curve drives a channel.
#[tauri::command]
pub async fn set_channel_curve(app: AppHandle, channel: String, curve: String) -> WriteResult {
    run_write(app, move |current| {
        fand_core::set_channel_curve(current, &channel, &curve).map_err(|e| e.to_string())
    })
    .await
}

/// Removes a curve. Fails (daemon-validated) if any channel or mix still
/// references it — the frontend should already be disabling this for
/// in-use curves, but the daemon is the backstop.
#[tauri::command]
pub async fn delete_curve(app: AppHandle, name: String) -> WriteResult {
    run_write(app, move |current| {
        fand_core::remove_curve(current, &name).map_err(|e| e.to_string())
    })
    .await
}
