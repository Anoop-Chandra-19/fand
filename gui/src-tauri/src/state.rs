//! The backend's config copy — the only one in the GUI process.
//!
//! The React side is a pure presentation layer: it never fetches or caches
//! config, it just renders the last `status` / `config` event it received.
//! Everything that used to make that hard (out-of-order responses, syncs
//! racing a reconnect, generation counters resetting with the daemon) is
//! resolved here instead, by three rules:
//!
//! 1. **Ordering by the mutex.** Every event that carries config is
//!    emitted while holding the one cache mutex, so the webview receives
//!    payloads in exactly the order the cache advanced.
//! 2. **Generations never cross instances.** The daemon stamps everything
//!    with a random per-process `instance` token; a generation number is
//!    only ever compared against the same instance's. A write that raced
//!    a daemon restart carries the dead instance's token and cannot
//!    advance the cache; a fetch that answers from a different instance
//!    than the frame in hand proves the stream is stale and triggers a
//!    reconnect instead of a cross-instance pairing.
//! 3. **Frames pair only with config that covers them.** A frame is
//!    emitted with the cached config iff that config is the same
//!    instance at the frame's generation or newer; otherwise with
//!    `config: None` for that frame. "Newer" is the accepted transient:
//!    within one instance, config may briefly run ahead of queued status
//!    frames (possibly by several generations after rapid writes) — the
//!    next computed frame catches up.
//!
//! The write/pump division of labour: only the pump may *establish* the
//! cache (decide which instance is current); writes may only advance the
//! generation within the instance the pump established, and are
//! serialized by the write gate in `curves.rs`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use fand_core::config::CurveConfig;
use fand_proto::client::Client;
use fand_proto::{ConfigVersion, Status};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::socket_path;

/// Read/write deadline for every daemon connection the GUI opens. A wedged
/// daemon (accepts, never answers) fails a request instead of hanging a
/// command — or the pump — forever.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct ChannelSettings {
    pub min_pwm: u8,
    pub smoothing_seconds: u64,
    pub offset_pwm: i16,
}

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

/// Everything the frontend knows about the daemon's config, in one piece —
/// curves, channel bindings, sensor names and per-channel settings can
/// never be from different generations.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigPayload {
    pub curves: BTreeMap<String, CurveInfo>,
    /// channel name → the curve it binds.
    pub channels: BTreeMap<String, String>,
    /// Already-configured sensor names, for graph-curve sensor pickers.
    pub sensors: Vec<String>,
    pub channel_settings: BTreeMap<String, ChannelSettings>,
    pub config_generation: u64,
    /// Which daemon lifetime the generation belongs to (see the module
    /// docs — generations are never compared across instances).
    pub instance: u64,
    /// The daemon's control-loop interval; the pump derives its stream
    /// wedge-detection timeout from it.
    pub tick_seconds: u64,
}

/// One status frame plus the newest same-instance config that covers it.
/// Usually that is exactly the config the frame was computed under; after
/// rapid writes it may transiently be ahead of a queued frame (same
/// instance, higher generation) — never behind, and never another daemon
/// lifetime's config.
#[derive(Debug, Clone, Serialize)]
pub struct StatusEvent {
    pub status: Status,
    /// None when no covering config is known this frame (fetch failed —
    /// the next frame retries).
    pub config: Option<ConfigPayload>,
}

/// The cached payload, managed by Tauri. None while disconnected or before
/// the first successful fetch of a connection.
#[derive(Default)]
pub struct SharedConfig(Mutex<Option<ConfigPayload>>);

pub(crate) fn payload_from_config(
    cfg: &fand_core::Config,
    version: ConfigVersion,
) -> ConfigPayload {
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

    let channel_settings = cfg
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
        .collect();

    ConfigPayload {
        curves,
        channels,
        sensors: cfg.sensors.keys().cloned().collect(),
        channel_settings,
        config_generation: version.generation,
        instance: version.instance,
        tick_seconds: cfg.daemon.tick_seconds,
    }
}

/// How a write's publish attempt landed. Only `Published` reached the
/// webview; the others are safe non-events for the cache, but
/// `StaleInstance` deserves a user-visible warning (the change applied to
/// a daemon that has since gone away — the *current* daemon may differ).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishOutcome {
    Published,
    /// No cache yet — the pump hasn't established which daemon is current;
    /// its first frame will deliver this config anyway.
    CacheUnestablished,
    /// The write's daemon is not the one the pump established.
    StaleInstance,
    /// Same instance, but the cache already advanced past this write.
    Superseded,
}

/// May a write advance the cache with `incoming`? Only within the
/// instance the pump established, and never backwards. An empty cache is
/// a refusal too: establishing which daemon is current is the pump's job,
/// and a write that lands before the pump's first frame simply waits for
/// it (its config arrives with that frame anyway).
fn classify_write(cache: &Option<ConfigPayload>, incoming: &ConfigPayload) -> PublishOutcome {
    match cache {
        None => PublishOutcome::CacheUnestablished,
        Some(c) if c.instance != incoming.instance => PublishOutcome::StaleInstance,
        Some(c) if incoming.config_generation < c.config_generation => PublishOutcome::Superseded,
        Some(_) => PublishOutcome::Published,
    }
}

/// Does the cache cover `status`? Requires the same instance — a cache
/// holding another daemon lifetime's config never covers anything, no
/// matter how large its generation number, which is what makes a stale
/// cache self-healing (the next frame refetches).
fn frame_covered(cache: &Option<ConfigPayload>, status: &Status) -> bool {
    cache.as_ref().is_some_and(|c| {
        c.instance == status.instance && c.config_generation >= status.config_generation
    })
}

/// What the pump should do with a fetch result, given the frame that
/// triggered the fetch. Pure — this is the whole install policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallDecision {
    Install,
    /// Keep the current cache (fetch failed, or a concurrent write already
    /// advanced it past the fetched generation).
    Keep,
    /// The fetch answered from a *different daemon* than the frame came
    /// from: the daemon restarted mid-frame, so the stream in hand is
    /// dead — drop the frame and reconnect instead of pairing across
    /// instances.
    StaleStream,
}

fn install_decision(
    cache: &Option<ConfigPayload>,
    fetched: &Result<ConfigPayload, String>,
    status: &Status,
) -> InstallDecision {
    let Ok(fetched) = fetched else {
        return InstallDecision::Keep;
    };
    if fetched.instance != status.instance {
        return InstallDecision::StaleStream;
    }
    let advanced_past = cache.as_ref().is_some_and(|c| {
        c.instance == fetched.instance && c.config_generation > fetched.config_generation
    });
    if advanced_past {
        InstallDecision::Keep
    } else {
        InstallDecision::Install
    }
}

/// Forget the cached config. The pump calls this on every new connection:
/// with instance tokens this is no longer load-bearing for correctness,
/// but it keeps a dead daemon's config from being paired with the first
/// frames of a new one while the first fetch is still in flight.
pub(crate) fn clear(app: &AppHandle) {
    let state = app.state::<SharedConfig>();
    *state.0.lock().unwrap() = None;
}

/// Called by a write command with the daemon-confirmed payload it read
/// back: advance the cache and tell the webview. Emitting under the lock
/// is deliberate — it serializes this with the pump's per-frame emit, so
/// config payloads reach the webview in cache order (the emit itself is
/// an in-process dispatch, not I/O). The returned outcome says whether it
/// landed; a non-`Published` outcome never touched the cache and the
/// pump's next frame carries the daemon's real state.
pub(crate) fn publish(app: &AppHandle, payload: ConfigPayload) -> PublishOutcome {
    let state = app.state::<SharedConfig>();
    let mut cache = state.0.lock().unwrap();
    let outcome = classify_write(&cache, &payload);
    if outcome == PublishOutcome::Published {
        *cache = Some(payload.clone());
        let _ = app.emit("config", &payload);
    }
    outcome
}

/// What the pump should do after a frame was handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameOutcome {
    /// Frame emitted; if the covering config is known, its tick interval
    /// (for the pump's wedge-detection deadline — derived only from config
    /// that actually covers the stream's frames).
    Emitted { tick_seconds: Option<u64> },
    /// The daemon restarted under this stream — reconnect immediately.
    StaleStream,
}

/// Called by the pump for every status frame: make sure the cache covers
/// the frame (fetching from the daemon if not), then emit the frame,
/// paired with the config *iff it covers the frame* — a frame is never
/// paired with config the daemon didn't serve it under, except the
/// accepted transient where the same instance's config runs ahead of a
/// queued frame (see `StatusEvent::config`). A failed fetch never touches
/// the cache (a concurrent write may just have advanced it); it emits
/// whatever honestly covers, or `config: None`, and the next frame
/// retries — level-triggered, like the stream itself.
///
/// The fetch happens *between* the two lock scopes, so a slow daemon
/// delays only the pump, never a write command's publish.
pub(crate) fn emit_frame(app: &AppHandle, status: Status) -> FrameOutcome {
    let state = app.state::<SharedConfig>();
    let covered = frame_covered(&state.0.lock().unwrap(), &status);
    let fetched = if covered { None } else { Some(fetch_payload()) };

    let mut cache = state.0.lock().unwrap();
    if let Some(fetched) = fetched {
        match install_decision(&cache, &fetched, &status) {
            InstallDecision::Install => *cache = Some(fetched.unwrap()),
            InstallDecision::Keep => {
                if let Err(e) = &fetched {
                    log_fetch_failure(e);
                }
            }
            InstallDecision::StaleStream => return FrameOutcome::StaleStream,
        }
    }
    let config = frame_covered(&cache, &status).then(|| cache.clone().unwrap());
    let tick_seconds = config.as_ref().map(|c| c.tick_seconds);
    let event = StatusEvent { status, config };
    let _ = app.emit("status", &event);
    FrameOutcome::Emitted { tick_seconds }
}

fn fetch_payload() -> Result<ConfigPayload, String> {
    let mut client =
        Client::connect_with_timeout(socket_path(), REQUEST_TIMEOUT).map_err(|e| e.to_string())?;
    let snap = client.get_config().map_err(|e| e.to_string())?;
    let cfg = fand_core::Config::from_toml_str(&snap.toml).map_err(|e| e.to_string())?;
    Ok(payload_from_config(&cfg, snap.version))
}

/// A persistently failing fetch would otherwise log every tick; once per
/// 30 s is enough to diagnose without flooding stderr.
fn log_fetch_failure(error: &str) {
    static LAST_LOG_SECS: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let last = LAST_LOG_SECS.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= 30
        && LAST_LOG_SECS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        eprintln!("fand-gui: config fetch failed (retrying every frame): {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(instance: u64, generation: u64) -> ConfigPayload {
        ConfigPayload {
            curves: BTreeMap::new(),
            channels: BTreeMap::new(),
            sensors: Vec::new(),
            channel_settings: BTreeMap::new(),
            config_generation: generation,
            instance,
            tick_seconds: 2,
        }
    }

    fn status(instance: u64, generation: u64) -> Status {
        Status {
            temps: BTreeMap::new(),
            channels: BTreeMap::new(),
            config_generation: generation,
            instance,
        }
    }

    /// The reviewed High: a write completed against a daemon that has
    /// since restarted. Its generation (50) dwarfs the new daemon's (1),
    /// but the instance token gives it away — it must not advance a cache
    /// the pump established from the new daemon, and the caller learns
    /// why (to warn the user).
    #[test]
    fn stale_write_from_dead_daemon_cannot_advance_cache() {
        let cache = Some(payload(0xB, 1));
        assert_eq!(
            classify_write(&cache, &payload(0xA, 50)),
            PublishOutcome::StaleInstance
        );
    }

    #[test]
    fn write_advances_only_forward_within_instance() {
        let cache = Some(payload(0xA, 5));
        assert_eq!(
            classify_write(&cache, &payload(0xA, 5)),
            PublishOutcome::Published
        );
        assert_eq!(
            classify_write(&cache, &payload(0xA, 6)),
            PublishOutcome::Published
        );
        assert_eq!(
            classify_write(&cache, &payload(0xA, 4)),
            PublishOutcome::Superseded
        );
    }

    /// Establishing the cache is the pump's job; a write that lands first
    /// (reconnect race) waits for the pump's frame instead.
    #[test]
    fn write_cannot_establish_an_empty_cache() {
        assert_eq!(
            classify_write(&None, &payload(0xA, 1)),
            PublishOutcome::CacheUnestablished
        );
    }

    /// Even if a wrong-instance payload somehow reached the cache, its
    /// huge generation would not count as covering the new daemon's
    /// frames — the pump refetches on the very next frame.
    #[test]
    fn wrong_instance_cache_never_covers_a_frame() {
        let cache = Some(payload(0xA, 50));
        assert!(!frame_covered(&cache, &status(0xB, 1)));
    }

    #[test]
    fn coverage_within_instance_is_by_generation() {
        let cache = Some(payload(0xA, 3));
        assert!(frame_covered(&cache, &status(0xA, 3)));
        assert!(
            frame_covered(&cache, &status(0xA, 2)),
            "cache may run ahead"
        );
        assert!(!frame_covered(&cache, &status(0xA, 4)));
        assert!(!frame_covered(&None, &status(0xA, 0)));
    }

    /// A fetch that answers from a different instance than the triggering
    /// frame proves the daemon restarted under the stream: never install,
    /// never pair — reconnect.
    #[test]
    fn fetch_from_other_instance_means_stale_stream() {
        let fetched = Ok(payload(0xB, 1));
        assert_eq!(
            install_decision(&None, &fetched, &status(0xA, 3)),
            InstallDecision::StaleStream
        );
        assert_eq!(
            install_decision(&Some(payload(0xA, 3)), &fetched, &status(0xA, 3)),
            InstallDecision::StaleStream
        );
    }

    /// A failed fetch keeps the cache exactly as it is — in particular it
    /// must not erase a covering payload a concurrent write installed
    /// while the fetch was in flight.
    #[test]
    fn fetch_failure_preserves_concurrently_advanced_cache() {
        let cache = Some(payload(0xA, 7));
        let failed: Result<ConfigPayload, String> = Err("connect refused".into());
        assert_eq!(
            install_decision(&cache, &failed, &status(0xA, 6)),
            InstallDecision::Keep
        );
        // The advanced cache still covers the frame, so the frame is
        // emitted with real config despite the failed fetch.
        assert!(frame_covered(&cache, &status(0xA, 6)));
    }

    /// Within the frame's instance, the pump installs its fetch unless a
    /// concurrent write already advanced the cache past it.
    #[test]
    fn pump_installs_within_instance_but_never_steps_back() {
        let frame = status(0xA, 3);
        assert_eq!(
            install_decision(&None, &Ok(payload(0xA, 3)), &frame),
            InstallDecision::Install
        );
        assert_eq!(
            install_decision(&Some(payload(0xA, 2)), &Ok(payload(0xA, 3)), &frame),
            InstallDecision::Install
        );
        assert_eq!(
            install_decision(&Some(payload(0xA, 4)), &Ok(payload(0xA, 3)), &frame),
            InstallDecision::Keep,
            "a write advanced the cache mid-fetch; the older fetch must not undo it"
        );
        // A leftover cache from a previous lifetime never blocks the
        // pump's install for the current one.
        assert_eq!(
            install_decision(&Some(payload(0xB, 50)), &Ok(payload(0xA, 3)), &frame),
            InstallDecision::Install
        );
    }
}
