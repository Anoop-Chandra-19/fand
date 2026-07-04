//! Startup wiring + the control loop. Per channel each tick:
//!
//! ```text
//! sensor temps → CurveTree::eval (smoothing + graph/mix/flat) → Ramp::step → pwm write
//! ```
//!
//! Any tick error (sensor read failure, implausible reading, failed write)
//! drives every controlled fan to full and exits nonzero — never loop on
//! stale data.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use fand_core::config::SensorConfig;
use fand_core::{window_ticks, Config, CurveTree, Kick, Ramp, RampConfig};
use fand_proto::{ChannelStatus, Command, Response, ResponseData, Status};

use crate::failsafe::SharedPaths;
use crate::hub::{EngineCommand, StatusHub};
use crate::hwmon::HwmonDevice;
use crate::nvml::Gpu;

/// On failure every controlled fan is driven to full before exiting.
const FAILSAFE_PWM: u8 = 255;
/// Readings at or outside these bounds mean a broken sensor, not a real
/// temperature (k10temp/NVML report well inside this window).
const TEMP_PLAUSIBLE_MIN_C: f64 = 0.0;
const TEMP_PLAUSIBLE_MAX_C: f64 = 115.0;
/// pwmN_enable value for manual control.
const MANUAL: u8 = 1;
/// pwmN_enable value for firmware auto (same value failsafe::FIRMWARE_AUTO
/// writes as text); used when a reload drops a channel from the config.
const FIRMWARE_AUTO: u8 = 5;

enum SensorSource {
    Hwmon { hwmon_name: String, input: String },
    Nvml { device_index: u32 },
}

struct Channel {
    name: String,
    hwmon_name: String,
    pwm_index: u32,
    /// The channel's bound curve, resolved into an owned tree (smoothing
    /// state lives inside the graph nodes).
    tree: CurveTree,
    ramp: Ramp,
    last_written: Option<u8>,
    /// Copied from config for override validation: the lowest PWM a client
    /// may pin this channel to (0 only when zero_rpm is opted in).
    min_pwm: u8,
    zero_rpm: bool,
}

/// A client-requested pin: the channel follows `pwm` (through the ramp)
/// instead of its curves until the deadline passes. Runtime-only state —
/// never persisted, dropped on daemon restart.
struct Override {
    pwm: u8,
    expires_at: Instant,
}

/// Longest allowed override TTL; a forgotten override must always hand
/// control back to the curves eventually.
const MAX_OVERRIDE_TTL_S: u64 = 3600;

pub struct Engine {
    devices: BTreeMap<String, HwmonDevice>,
    gpu: Option<Gpu>,
    sensors: BTreeMap<String, SensorSource>,
    channels: Vec<Channel>,
    overrides: BTreeMap<String, Override>,
    tick: Duration,
    dry_run: bool,
    /// The applied config as TOML text — kept verbatim (comments included)
    /// so `get_config` returns exactly what the user wrote.
    config_toml: String,
    /// Where hwmon devices get re-resolved on reload (fake sysfs in tests).
    hwmon_root: PathBuf,
    /// Restore-on-exit list shared with the failsafe guard and panic hook;
    /// None in dry-run. Reload updates it when the channel set changes.
    failsafe_paths: Option<SharedPaths>,
    /// The file `ReloadConfig` re-reads; None when the engine was built
    /// from a string only (tests).
    config_path: Option<PathBuf>,
}

impl Engine {
    /// Resolve hardware and build per-channel state from a **validated**
    /// config (`toml_text` is the raw text `cfg` was parsed from). Fails
    /// loudly on: missing hwmon chip, NVML init failure, or a configured
    /// channel whose fan shows no tach under firmware control.
    pub fn from_config(
        cfg: &Config,
        toml_text: &str,
        hwmon_root: &Path,
        dry_run: bool,
    ) -> Result<Self> {
        Self::build(cfg, toml_text, hwmon_root, dry_run, &BTreeSet::new())
    }

    /// `from_config` plus a probe exemption: channels in
    /// `already_controlled` skip the 0-RPM liveness check, because during a
    /// reload *we* hold them — a zero-RPM fan we parked legitimately reads
    /// 0 and must not be mistaken for a dead header.
    fn build(
        cfg: &Config,
        toml_text: &str,
        hwmon_root: &Path,
        dry_run: bool,
        already_controlled: &BTreeSet<String>,
    ) -> Result<Self> {
        let mut devices = BTreeMap::new();
        let referenced_chips = cfg
            .sensors
            .values()
            .filter_map(|s| match s {
                SensorConfig::Hwmon { hwmon_name, .. } => Some(hwmon_name),
                SensorConfig::Nvml { .. } => None,
            })
            .chain(cfg.channels.values().map(|ch| &ch.hwmon_name));
        for name in referenced_chips {
            if !devices.contains_key(name) {
                devices.insert(name.clone(), HwmonDevice::find_by_name(hwmon_root, name)?);
            }
        }

        let needs_nvml = cfg
            .sensors
            .values()
            .any(|s| matches!(s, SensorConfig::Nvml { .. }));
        let gpu = if needs_nvml { Some(Gpu::init()?) } else { None };

        let sensors = cfg
            .sensors
            .iter()
            .map(|(name, s)| {
                let source = match s {
                    SensorConfig::Hwmon { hwmon_name, input } => SensorSource::Hwmon {
                        hwmon_name: hwmon_name.clone(),
                        input: input.clone(),
                    },
                    SensorConfig::Nvml { device_index } => SensorSource::Nvml {
                        device_index: *device_index,
                    },
                };
                (name.clone(), source)
            })
            .collect();

        let tick_seconds = cfg.daemon.tick_seconds;
        let mut channels = Vec::new();
        for (name, ch) in &cfg.channels {
            let pwm_index: u32 = name
                .strip_prefix("pwm")
                .and_then(|s| s.parse().ok())
                .with_context(|| format!("channel `{name}`: name must be pwmN"))?;
            let dev = &devices[&ch.hwmon_name];

            // Liveness probe (before anything is written anywhere): a dead
            // header reads 0 RPM under firmware control.
            let rpm = dev
                .read_fan_rpm(pwm_index)
                .with_context(|| format!("channel `{name}`: liveness probe"))?;
            if rpm == 0 && !already_controlled.contains(name) {
                bail!(
                    "channel `{name}`: fan{pwm_index}_input reads 0 RPM under firmware \
                     control — dead header, refusing to control it"
                );
            }

            let window = window_ticks(ch.smoothing_seconds, tick_seconds);
            let tree = CurveTree::build(&cfg.curves, &ch.curve, window)
                .with_context(|| format!("channel `{name}`: resolving curve `{}`", ch.curve))?;

            let kick = if ch.zero_rpm {
                Some(Kick {
                    pwm: ch
                        .kick_pwm
                        .with_context(|| format!("channel `{name}`: zero_rpm without kick_pwm"))?,
                    ticks: window_ticks(
                        ch.kick_seconds.with_context(|| {
                            format!("channel `{name}`: zero_rpm without kick_seconds")
                        })?,
                        tick_seconds,
                    ) as u32,
                })
            } else {
                None
            };

            // Start the ramp from whatever duty firmware left the fan at,
            // so taking over does not step the speed.
            let initial = dev
                .read_pwm(pwm_index)
                .with_context(|| format!("channel `{name}`: reading initial pwm"))?;
            let ramp = Ramp::new(
                RampConfig {
                    min_pwm: ch.min_pwm,
                    max_step_up: ch.max_step_up,
                    max_step_down: ch.max_step_down,
                    deadband: ch.deadband,
                    kick,
                },
                initial,
            );

            eprintln!(
                "fand: channel {name} on {}: fan{pwm_index} at {rpm} RPM, pwm {initial}",
                ch.hwmon_name
            );
            channels.push(Channel {
                name: name.clone(),
                hwmon_name: ch.hwmon_name.clone(),
                pwm_index,
                tree,
                ramp,
                last_written: None,
                min_pwm: ch.min_pwm,
                zero_rpm: ch.zero_rpm,
            });
        }

        if channels.is_empty() {
            bail!("config has no channels — nothing to control");
        }

        Ok(Self {
            devices,
            gpu,
            sensors,
            channels,
            overrides: BTreeMap::new(),
            tick: Duration::from_secs(tick_seconds),
            dry_run,
            config_toml: toml_text.to_string(),
            hwmon_root: hwmon_root.to_path_buf(),
            failsafe_paths: None,
            config_path: None,
        })
    }

    /// Hand the engine the restore-on-exit list it shares with the
    /// failsafe guard, so reloads can keep it in sync with the channel set.
    pub fn set_failsafe_paths(&mut self, paths: SharedPaths) {
        self.failsafe_paths = Some(paths);
    }

    /// The file `ReloadConfig` and SIGHUP re-read.
    pub fn set_config_source(&mut self, path: PathBuf) {
        self.config_path = Some(path);
    }

    /// pwmN_enable paths for every controlled channel — what the failsafe
    /// guard and panic hook must restore.
    pub fn enable_paths(&self) -> Vec<PathBuf> {
        self.channels
            .iter()
            .map(|ch| self.devices[&ch.hwmon_name].pwm_enable_path(ch.pwm_index))
            .collect()
    }

    /// Switch configured channels to manual control. Call only after the
    /// failsafe guard and panic hook are in place.
    pub fn take_control(&self) -> Result<()> {
        for ch in &self.channels {
            let dev = &self.devices[&ch.hwmon_name];
            let original = dev.read_pwm_enable(ch.pwm_index)?;
            if self.dry_run {
                eprintln!(
                    "fand: [dry-run] would set pwm{}_enable {original} → {MANUAL}",
                    ch.pwm_index
                );
            } else {
                eprintln!(
                    "fand: taking manual control of {} (pwm{}_enable {original} → {MANUAL})",
                    ch.name, ch.pwm_index
                );
                dev.write_pwm_enable(ch.pwm_index, MANUAL)?;
            }
        }
        Ok(())
    }

    /// Re-read the config file and hot-apply it (ReloadConfig / SIGHUP).
    pub fn reload_from_disk(&mut self) -> Result<()> {
        let path = self
            .config_path
            .clone()
            .context("daemon has no config path to reload from")?;
        let toml = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        self.reload(&toml)
    }

    /// Apply a new config without restarting. Order matters for safety:
    /// nothing is written to hardware until the whole new config has parsed,
    /// validated, and resolved against live hardware; the failsafe list is
    /// widened to the union of old+new channels before any enable write, so
    /// a crash mid-transition still restores everything. On error the
    /// running engine is left untouched.
    pub fn reload(&mut self, toml_text: &str) -> Result<()> {
        let cfg = Config::from_toml_str(toml_text).map_err(|e| anyhow::anyhow!("{e}"))?;
        let controlled: BTreeSet<String> =
            self.channels.iter().map(|c| c.name.clone()).collect();
        let mut new = Engine::build(&cfg, toml_text, &self.hwmon_root, self.dry_run, &controlled)?;

        if let Some(shared) = &self.failsafe_paths {
            let mut union = self.enable_paths();
            for p in new.enable_paths() {
                if !union.contains(&p) {
                    union.push(p);
                }
            }
            shared.set(union);
        }

        // Take manual control of channels this config adds. On failure,
        // hand back what was already switched and keep the old config.
        let mut switched: Vec<usize> = Vec::new();
        for (i, ch) in new.channels.iter().enumerate() {
            if controlled.contains(&ch.name) {
                continue;
            }
            let dev = &new.devices[&ch.hwmon_name];
            if self.dry_run {
                eprintln!("fand: [dry-run] would take manual control of {}", ch.name);
                continue;
            }
            match dev.write_pwm_enable(ch.pwm_index, MANUAL) {
                Ok(()) => {
                    eprintln!("fand: taking manual control of {} (reload)", ch.name);
                    switched.push(i);
                }
                Err(e) => {
                    for &j in &switched {
                        let c = &new.channels[j];
                        let _ = new.devices[&c.hwmon_name].write_pwm_enable(c.pwm_index, FIRMWARE_AUTO);
                    }
                    if let Some(shared) = &self.failsafe_paths {
                        shared.set(self.enable_paths());
                    }
                    return Err(e).with_context(|| {
                        format!(
                            "taking control of new channel `{}` — reload aborted, \
                             previous config still active",
                            ch.name
                        )
                    });
                }
            }
        }

        // Hand back channels the new config drops.
        let new_names: BTreeSet<&str> = new.channels.iter().map(|c| c.name.as_str()).collect();
        for ch in &self.channels {
            if new_names.contains(ch.name.as_str()) {
                continue;
            }
            if self.dry_run {
                eprintln!("fand: [dry-run] would restore firmware auto on {}", ch.name);
            } else {
                match self.devices[&ch.hwmon_name].write_pwm_enable(ch.pwm_index, FIRMWARE_AUTO) {
                    Ok(()) => eprintln!(
                        "fand: {}: dropped from config — firmware auto restored",
                        ch.name
                    ),
                    Err(e) => eprintln!(
                        "fand: FAILED to restore firmware auto on dropped channel {}: {e:#}",
                        ch.name
                    ),
                }
            }
        }

        // Overrides survive when their channel still exists and their pin
        // is still legal under the new floors.
        for (name, o) in std::mem::take(&mut self.overrides) {
            match new.channels.iter().find(|c| c.name == name) {
                Some(ch) if o.pwm >= (if ch.zero_rpm { 0 } else { ch.min_pwm }) => {
                    new.overrides.insert(name, o);
                }
                _ => eprintln!("fand: {name}: active override dropped by config reload"),
            }
        }

        new.failsafe_paths = self.failsafe_paths.clone();
        new.config_path = self.config_path.clone();
        *self = new;
        if let Some(shared) = &self.failsafe_paths {
            shared.set(self.enable_paths());
        }
        eprintln!(
            "fand: config applied: {} sensor(s), {} curve(s), {} channel(s)",
            cfg.sensors.len(),
            cfg.curves.len(),
            cfg.channels.len()
        );
        Ok(())
    }

    /// Hot-apply a client-supplied config, then persist it as the config
    /// file (skipped in dry-run — a dev daemon must not rewrite the real
    /// file). Apply-then-persist order means a config that fails against
    /// live hardware never lands on disk.
    fn set_config(&mut self, toml_text: &str) -> Result<()> {
        self.reload(toml_text)?;
        if self.dry_run {
            eprintln!("fand: [dry-run] config applied in memory only, not persisted");
            return Ok(());
        }
        if let Some(path) = self.config_path.clone() {
            persist_config(&path, toml_text).with_context(|| {
                format!(
                    "config APPLIED to the running daemon but could not be \
                     persisted to {} — a restart will revert it",
                    path.display()
                )
            })?;
            eprintln!("fand: config persisted to {}", path.display());
        }
        Ok(())
    }

    /// Tick until `shutdown` is set (SIGTERM/SIGINT), publishing a status
    /// snapshot to `hub` each tick and serving client `commands` between
    /// ticks. On any tick error: fans to full, then propagate — the
    /// caller's guard restores auto.
    pub fn run(
        &mut self,
        shutdown: &AtomicBool,
        reload: &AtomicBool,
        hub: &StatusHub,
        commands: &Receiver<EngineCommand>,
    ) -> Result<()> {
        while !shutdown.load(Ordering::Relaxed) {
            // swap(false) reads and clears the SIGHUP flag in one step.
            if reload.swap(false, Ordering::Relaxed) {
                eprintln!("fand: SIGHUP — reloading config");
                if let Err(e) = self.reload_from_disk() {
                    eprintln!("fand: reload failed: {e:#} — keeping previous config");
                }
            }
            match self.tick_once() {
                Ok(status) => hub.publish(status),
                Err(e) => {
                    self.failsafe();
                    return Err(e.context(
                        "control tick failed — drove fans to full; firmware auto restored on exit",
                    ));
                }
            }
            self.idle(shutdown, reload, commands);
        }
        eprintln!("fand: shutdown requested");
        Ok(())
    }

    /// Wait out the tick interval, serving client commands as they arrive.
    /// Returns early when a command changed control targets (so the caller
    /// re-ticks immediately) or shutdown is requested. The 200 ms cap on
    /// each wait bounds how long a signal can go unnoticed.
    fn idle(&mut self, shutdown: &AtomicBool, reload: &AtomicBool, commands: &Receiver<EngineCommand>) {
        const SLICE: Duration = Duration::from_millis(200);
        let deadline = Instant::now() + self.tick;
        while !shutdown.load(Ordering::Relaxed) && !reload.load(Ordering::Relaxed) {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            if remaining.is_zero() {
                return;
            }
            match commands.recv_timeout(remaining.min(SLICE)) {
                Ok(cmd) => {
                    if self.handle_command(cmd) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    // No senders left (server thread gone) — plain sleep so
                    // this loop does not spin.
                    thread::sleep(remaining.min(SLICE));
                }
            }
        }
    }

    /// Handle one client command on the engine thread; returns true when
    /// control targets changed and the loop should re-tick immediately.
    fn handle_command(&mut self, cmd: EngineCommand) -> bool {
        let (response, retick) = match cmd.cmd {
            Command::GetConfig => (
                Response::ok(ResponseData::Config {
                    toml: self.config_toml.clone(),
                }),
                false,
            ),
            Command::SetOverride {
                channel,
                pwm,
                ttl_seconds,
            } => match self.set_override(&channel, pwm, ttl_seconds) {
                Ok(ttl) => {
                    eprintln!("fand: {channel}: override pwm {pwm} for {ttl}s");
                    (Response::ok_empty(), true)
                }
                Err(e) => (Response::err(format!("{e:#}")), false),
            },
            Command::ClearOverride { channel } => match self.clear_override(&channel) {
                Ok(existed) => {
                    if existed {
                        eprintln!("fand: {channel}: override cleared — back to curve");
                    }
                    (Response::ok_empty(), existed)
                }
                Err(e) => (Response::err(format!("{e:#}")), false),
            },
            Command::ReloadConfig => match self.reload_from_disk() {
                Ok(()) => (Response::ok_empty(), true),
                Err(e) => (Response::err(format!("{e:#}")), false),
            },
            Command::SetConfig { toml } => match self.set_config(&toml) {
                Ok(()) => (Response::ok_empty(), true),
                Err(e) => (Response::err(format!("{e:#}")), false),
            },
            // GetStatus/SubscribeStatus are answered by the server threads
            // straight from the hub and never reach this channel.
            Command::GetStatus | Command::SubscribeStatus => {
                (Response::err("internal: status commands are not engine commands"), false)
            }
        };
        // A dead client is the connection thread's problem, not ours.
        let _ = cmd.reply.send(response);
        retick
    }

    /// Validate and store an override. The floor check is the safety-
    /// critical part: a channel can never be pinned below its min_pwm
    /// (below 0 RPM territory for the pwm1 pump) unless it explicitly
    /// opted into zero_rpm. Returns the clamped TTL actually applied.
    fn set_override(&mut self, channel: &str, pwm: u8, ttl_seconds: u64) -> Result<u64> {
        let ch = self.find_channel(channel)?;
        let floor = if ch.zero_rpm { 0 } else { ch.min_pwm };
        if pwm < floor {
            bail!(
                "pwm {pwm} is below channel `{channel}`'s floor {floor} — refusing \
                 (overrides cannot push a fan into its stall region)"
            );
        }
        let ttl = ttl_seconds.clamp(1, MAX_OVERRIDE_TTL_S);
        self.overrides.insert(
            channel.to_string(),
            Override {
                pwm,
                expires_at: Instant::now() + Duration::from_secs(ttl),
            },
        );
        Ok(ttl)
    }

    /// Ok(true) if an override was removed, Ok(false) if the channel exists
    /// but had none; unknown channels are an error (likely a typo).
    fn clear_override(&mut self, channel: &str) -> Result<bool> {
        self.find_channel(channel)?;
        Ok(self.overrides.remove(channel).is_some())
    }

    fn find_channel(&self, channel: &str) -> Result<&Channel> {
        self.channels.iter().find(|c| c.name == channel).with_context(|| {
            let known: Vec<&str> = self.channels.iter().map(|c| c.name.as_str()).collect();
            format!(
                "unknown channel `{channel}` (configured: {})",
                known.join(", ")
            )
        })
    }

    fn tick_once(&mut self) -> Result<Status> {
        let temps = read_temps(&self.sensors, &self.devices, self.gpu.as_ref())?;
        let now = Instant::now();
        let mut channel_status = BTreeMap::new();
        for ch in &mut self.channels {
            // The curve tree is evaluated even under an override: the
            // smoothing windows stay warm, and status keeps reporting what
            // the channel would do on its own.
            let raw_target = ch
                .tree
                .eval(&temps)
                .with_context(|| format!("channel `{}`", ch.name))?;

            let (ramp_target, mode, override_remaining_s) = match self.overrides.get(&ch.name) {
                Some(o) if now >= o.expires_at => {
                    eprintln!("fand: {}: override expired — back to curve", ch.name);
                    self.overrides.remove(&ch.name);
                    (raw_target, "curve", None)
                }
                Some(o) => {
                    let remaining = o.expires_at.duration_since(now).as_secs();
                    (o.pwm, "override", Some(remaining.max(1)))
                }
                None => (raw_target, "curve", None),
            };
            let pwm = ch.ramp.step(ramp_target);

            if ch.last_written != Some(pwm) {
                if self.dry_run {
                    eprintln!(
                        "fand: [dry-run] {}: would write pwm {pwm} (curve target {raw_target})",
                        ch.name
                    );
                } else {
                    self.devices[&ch.hwmon_name].write_pwm(ch.pwm_index, pwm)?;
                    eprintln!("fand: {}: pwm {pwm} (curve target {raw_target})", ch.name);
                }
                ch.last_written = Some(pwm);
            }

            let rpm = self.devices[&ch.hwmon_name]
                .read_fan_rpm(ch.pwm_index)
                .with_context(|| format!("channel `{}`: reading rpm", ch.name))?;
            channel_status.insert(
                ch.name.clone(),
                ChannelStatus {
                    rpm,
                    current_pwm: pwm,
                    target_pwm: raw_target,
                    mode: mode.to_string(),
                    override_remaining_s,
                },
            );
        }
        Ok(Status {
            temps,
            channels: channel_status,
        })
    }

    /// Best effort: full duty on every controlled channel. Restoring
    /// pwmN_enable is the guard's job, after this.
    fn failsafe(&self) {
        if self.dry_run {
            eprintln!("fand: [dry-run] would drive all controlled fans to full");
            return;
        }
        eprintln!("fand: FAILSAFE — driving all controlled fans to pwm {FAILSAFE_PWM}");
        for ch in &self.channels {
            if let Err(e) = self.devices[&ch.hwmon_name].write_pwm(ch.pwm_index, FAILSAFE_PWM) {
                eprintln!("fand: FAILED failsafe write on {}: {e:#}", ch.name);
            }
        }
    }
}

/// Write the config file safely: previous version saved as `.bak`, new
/// text written to a temp file first, then renamed into place — a crash
/// mid-write can never leave a half-written config.
fn persist_config(path: &Path, toml_text: &str) -> Result<()> {
    if path.exists() {
        let bak = path.with_extension("toml.bak");
        std::fs::copy(path, &bak)
            .with_context(|| format!("backing up to {}", bak.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, toml_text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

/// Read every configured sensor, applying the plausibility window. Any
/// error here is a sensor failure and must escalate to the failsafe.
fn read_temps(
    sensors: &BTreeMap<String, SensorSource>,
    devices: &BTreeMap<String, HwmonDevice>,
    gpu: Option<&Gpu>,
) -> Result<BTreeMap<String, f64>> {
    let mut temps = BTreeMap::new();
    for (name, source) in sensors {
        let temp = match source {
            SensorSource::Hwmon { hwmon_name, input } => {
                devices[hwmon_name].read_temp_c(input)
            }
            SensorSource::Nvml { device_index } => gpu
                .context("NVML sensor configured but NVML not initialized")?
                .read_temp_c(*device_index),
        }
        .with_context(|| format!("sensor `{name}`"))?;
        if temp <= TEMP_PLAUSIBLE_MIN_C || temp >= TEMP_PLAUSIBLE_MAX_C {
            bail!("sensor `{name}`: implausible reading {temp:.1} °C");
        }
        temps.insert(name.clone(), temp);
    }
    Ok(temps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failsafe::FailsafeGuard;
    use std::fs;

    /// Fake /sys/class/hwmon with an nct6799 (fan1 + fan2 live) and a
    /// k10temp.
    fn fake_sysfs() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let nct = dir.path().join("hwmon0");
        fs::create_dir(&nct).unwrap();
        fs::write(nct.join("name"), "nct6799\n").unwrap();
        fs::write(nct.join("fan1_input"), "900\n").unwrap();
        fs::write(nct.join("pwm1"), "100\n").unwrap();
        fs::write(nct.join("pwm1_enable"), "5\n").unwrap();
        fs::write(nct.join("fan2_input"), "789\n").unwrap();
        fs::write(nct.join("pwm2"), "128\n").unwrap();
        fs::write(nct.join("pwm2_enable"), "5\n").unwrap();
        let k10 = dir.path().join("hwmon1");
        fs::create_dir(&k10).unwrap();
        fs::write(k10.join("name"), "k10temp\n").unwrap();
        fs::write(k10.join("temp1_input"), "54000\n").unwrap();
        dir
    }

    // smoothing_seconds = tick_seconds → 1-tick window, so smoothing is a
    // pass-through and expected pwm values are easy to compute by hand.
    const TEST_CONFIG: &str = r#"
        [daemon]
        tick_seconds = 2

        [sensors.cpu]
        kind = "hwmon"
        hwmon_name = "k10temp"
        input = "temp1_input"

        [curves.c]
        kind = "graph"
        sensor = "cpu"
        points = [[40, 80], [70, 200]]

        [channels.pwm2]
        hwmon_name = "nct6799"
        curve = "c"
        min_pwm = 70
        smoothing_seconds = 2
    "#;

    fn engine(root: &tempfile::TempDir) -> Engine {
        let cfg = Config::from_toml_str(TEST_CONFIG).unwrap();
        Engine::from_config(&cfg, TEST_CONFIG, root.path(), false).unwrap()
    }

    fn read(root: &tempfile::TempDir, file: &str) -> String {
        fs::read_to_string(root.path().join("hwmon0").join(file)).unwrap()
    }

    #[test]
    fn builds_from_config_against_fake_sysfs() {
        let root = fake_sysfs();
        let e = engine(&root);
        assert_eq!(e.channels.len(), 1);
        assert_eq!(e.channels[0].pwm_index, 2);
    }

    #[test]
    fn dead_channel_is_refused() {
        let root = fake_sysfs();
        fs::write(root.path().join("hwmon0/fan2_input"), "0\n").unwrap();
        let cfg = Config::from_toml_str(TEST_CONFIG).unwrap();
        let Err(err) = Engine::from_config(&cfg, TEST_CONFIG, root.path(), false) else {
            panic!("dead channel must be refused");
        };
        assert!(err.to_string().contains("dead header"), "{err:#}");
    }

    #[test]
    fn missing_chip_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let cfg = Config::from_toml_str(TEST_CONFIG).unwrap();
        assert!(Engine::from_config(&cfg, TEST_CONFIG, root.path(), false).is_err());
    }

    #[test]
    fn take_control_writes_manual_and_guard_restores_auto() {
        let root = fake_sysfs();
        let e = engine(&root);
        e.take_control().unwrap();
        assert_eq!(read(&root, "pwm2_enable"), "1");

        drop(FailsafeGuard::new(SharedPaths::new(e.enable_paths())));
        assert_eq!(read(&root, "pwm2_enable"), "5");
    }

    #[test]
    fn tick_writes_curve_pwm() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        // 54 °C on (40,80)-(70,200) interpolates to 136; ramp starts at the
        // firmware pwm 128 and 136 is within one max_step_up.
        e.tick_once().unwrap();
        assert_eq!(read(&root, "pwm2"), "136");
    }

    #[test]
    fn tick_evaluates_mix_curve_tree() {
        let root = fake_sysfs();
        // Two graph curves on the same sensor with different shapes; the
        // mix takes the max of their outputs. At 54 °C: c → 136, hot → 156.
        let mix_config = TEST_CONFIG.replace(
            "curve = \"c\"",
            "curve = \"m\"",
        ) + r#"
        [curves.hot]
        kind = "graph"
        sensor = "cpu"
        points = [[40, 100], [70, 220]]

        [curves.m]
        kind = "mix"
        curves = ["c", "hot"]
    "#;
        let cfg = Config::from_toml_str(&mix_config).unwrap();
        let mut e = Engine::from_config(&cfg, &mix_config, root.path(), false).unwrap();
        let status = e.tick_once().unwrap();
        assert_eq!(status.channels["pwm2"].target_pwm, 156);
        // Ramp starts at firmware pwm 128, limited to one max_step_up of 10.
        assert_eq!(read(&root, "pwm2"), "138");
    }

    #[test]
    fn tick_reports_status() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        let status = e.tick_once().unwrap();
        assert_eq!(status.temps["cpu"], 54.0);
        let ch = &status.channels["pwm2"];
        assert_eq!(ch.rpm, 789);
        assert_eq!(ch.current_pwm, 136);
        assert_eq!(ch.target_pwm, 136);
        assert_eq!(ch.mode, "curve");
    }

    #[test]
    fn steady_state_stops_rewriting() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        e.tick_once().unwrap();
        assert_eq!(e.channels[0].last_written, Some(136));
        // Same temp again: pwm unchanged, and last_written dedupes the write.
        fs::write(root.path().join("hwmon0/pwm2"), "0\n").unwrap();
        e.tick_once().unwrap();
        assert_eq!(read(&root, "pwm2"), "0\n", "no rewrite expected");
    }

    #[test]
    fn implausible_low_temp_is_sensor_failure() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        fs::write(root.path().join("hwmon1/temp1_input"), "-5000\n").unwrap();
        let err = e.tick_once().unwrap_err();
        assert!(err.to_string().contains("implausible"), "{err:#}");
    }

    #[test]
    fn implausible_high_temp_is_sensor_failure() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        fs::write(root.path().join("hwmon1/temp1_input"), "120000\n").unwrap();
        assert!(e.tick_once().is_err());
    }

    #[test]
    fn unreadable_sensor_is_sensor_failure() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        fs::remove_file(root.path().join("hwmon1/temp1_input")).unwrap();
        assert!(e.tick_once().is_err());
    }

    #[test]
    fn get_config_returns_verbatim_toml() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let retick = e.handle_command(EngineCommand {
            cmd: Command::GetConfig,
            reply: reply_tx,
        });
        assert!(!retick, "get_config must not force a re-tick");
        let resp = reply_rx.try_recv().unwrap();
        assert!(resp.ok);
        let Some(ResponseData::Config { toml }) = resp.data else {
            panic!("expected config data, got {resp:?}");
        };
        assert_eq!(toml, TEST_CONFIG);
    }

    #[test]
    fn unimplemented_command_gets_error_reply() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        e.handle_command(EngineCommand {
            cmd: Command::ReloadConfig,
            reply: reply_tx,
        });
        assert!(!reply_rx.try_recv().unwrap().ok);
    }

    #[test]
    fn override_pins_channel_and_reports_mode() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        e.tick_once().unwrap(); // settle at curve value 136
        let ttl = e.set_override("pwm2", 200, 60).unwrap();
        assert_eq!(ttl, 60);
        let status = e.tick_once().unwrap();
        let ch = &status.channels["pwm2"];
        // Ramp limits the step: 136 + max_step_up(10) = 146, heading to 200.
        assert_eq!(ch.current_pwm, 146);
        assert_eq!(ch.mode, "override");
        assert!(ch.override_remaining_s.is_some());
        // target_pwm still shows the curve's own opinion.
        assert_eq!(ch.target_pwm, 136);
    }

    #[test]
    fn expired_override_returns_to_curve() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        e.tick_once().unwrap();
        e.overrides.insert(
            "pwm2".into(),
            Override {
                pwm: 200,
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );
        let status = e.tick_once().unwrap();
        let ch = &status.channels["pwm2"];
        assert_eq!(ch.mode, "curve");
        assert_eq!(ch.override_remaining_s, None);
        assert_eq!(ch.current_pwm, 136);
        assert!(e.overrides.is_empty(), "expired override must be dropped");
    }

    #[test]
    fn override_below_floor_is_rejected() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        // TEST_CONFIG pwm2 has min_pwm 70 and no zero_rpm.
        let err = e.set_override("pwm2", 50, 60).unwrap_err();
        assert!(err.to_string().contains("floor 70"), "{err:#}");
        assert!(e.overrides.is_empty());
    }

    #[test]
    fn override_to_zero_allowed_only_with_zero_rpm() {
        let root = fake_sysfs();
        let zero_rpm_config = TEST_CONFIG.replace(
            "min_pwm = 70",
            "min_pwm = 70\nzero_rpm = true\nkick_pwm = 100\nkick_seconds = 4",
        );
        let cfg = Config::from_toml_str(&zero_rpm_config).unwrap();
        let mut e = Engine::from_config(&cfg, &zero_rpm_config, root.path(), false).unwrap();
        e.set_override("pwm2", 0, 60).expect("zero_rpm channel may be pinned to 0");
    }

    #[test]
    fn override_ttl_is_clamped() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        assert_eq!(e.set_override("pwm2", 100, 0).unwrap(), 1);
        assert_eq!(e.set_override("pwm2", 100, 999_999).unwrap(), 3600);
    }

    #[test]
    fn override_unknown_channel_is_rejected() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        let err = e.set_override("pwm9", 100, 60).unwrap_err();
        assert!(err.to_string().contains("unknown channel"), "{err:#}");
        assert!(e.clear_override("pwm9").is_err());
    }

    #[test]
    fn clear_override_restores_curve_mode() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        e.tick_once().unwrap();
        e.set_override("pwm2", 200, 60).unwrap();
        assert!(e.clear_override("pwm2").unwrap());
        assert!(!e.clear_override("pwm2").unwrap(), "second clear is a no-op");
        let status = e.tick_once().unwrap();
        assert_eq!(status.channels["pwm2"].mode, "curve");
    }

    /// TEST_CONFIG plus a pwm1 channel (the fake sysfs has both headers).
    const TEST_CONFIG_TWO_CHANNELS: &str = r#"
        [daemon]
        tick_seconds = 2

        [sensors.cpu]
        kind = "hwmon"
        hwmon_name = "k10temp"
        input = "temp1_input"

        [curves.c]
        kind = "graph"
        sensor = "cpu"
        points = [[40, 80], [70, 200]]

        [channels.pwm1]
        hwmon_name = "nct6799"
        curve = "c"
        min_pwm = 80
        smoothing_seconds = 2

        [channels.pwm2]
        hwmon_name = "nct6799"
        curve = "c"
        min_pwm = 70
        smoothing_seconds = 2
    "#;

    #[test]
    fn reload_applies_new_curve() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        e.tick_once().unwrap();
        assert_eq!(read(&root, "pwm2"), "136");
        // Shift the curve up: 54 °C on (40,90)-(70,210) → 146.
        let hotter = TEST_CONFIG.replace("[[40, 80], [70, 200]]", "[[40, 90], [70, 210]]");
        e.reload(&hotter).unwrap();
        assert_eq!(e.config_toml, hotter, "get_config must serve the new text");
        e.tick_once().unwrap();
        assert_eq!(read(&root, "pwm2"), "146");
    }

    #[test]
    fn reload_rejects_bad_toml_and_keeps_running() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        e.tick_once().unwrap();
        assert!(e.reload("this is not toml").is_err());
        assert!(e.reload(&TEST_CONFIG.replace("min_pwm = 70", "min_pwm = 10")).is_err());
        assert_eq!(e.config_toml, TEST_CONFIG, "old config must stay applied");
        e.tick_once().unwrap();
    }

    #[test]
    fn reload_added_channel_is_probed_and_controlled() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        e.take_control().unwrap();
        e.reload(TEST_CONFIG_TWO_CHANNELS).unwrap();
        assert_eq!(read(&root, "pwm1_enable"), "1", "new channel under manual control");
        e.tick_once().unwrap();
        // 54 °C → 136 target; pwm1 ramps from its firmware value 100 by 10.
        assert_eq!(read(&root, "pwm1"), "110");
    }

    #[test]
    fn reload_dead_added_channel_is_refused_and_rolled_back() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        e.take_control().unwrap();
        fs::write(root.path().join("hwmon0/fan1_input"), "0\n").unwrap();
        let err = e.reload(TEST_CONFIG_TWO_CHANNELS).unwrap_err();
        assert!(err.to_string().contains("dead header"), "{err:#}");
        assert_eq!(read(&root, "pwm1_enable"), "5\n", "must stay firmware-controlled");
        e.tick_once().unwrap();
        assert_eq!(e.channels.len(), 1, "old config still active");
    }

    #[test]
    fn reload_zero_rpm_on_controlled_channel_is_not_a_dead_header() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        e.take_control().unwrap();
        // We hold the channel and (hypothetically) parked it: 0 RPM is
        // legitimate and must not fail the reload probe.
        fs::write(root.path().join("hwmon0/fan2_input"), "0\n").unwrap();
        e.reload(TEST_CONFIG).unwrap();
    }

    #[test]
    fn reload_removed_channel_returns_to_firmware() {
        let root = fake_sysfs();
        let cfg = Config::from_toml_str(TEST_CONFIG_TWO_CHANNELS).unwrap();
        let mut e =
            Engine::from_config(&cfg, TEST_CONFIG_TWO_CHANNELS, root.path(), false).unwrap();
        e.take_control().unwrap();
        assert_eq!(read(&root, "pwm1_enable"), "1");
        e.reload(TEST_CONFIG).unwrap();
        assert_eq!(read(&root, "pwm1_enable"), "5", "dropped channel handed back");
        assert_eq!(read(&root, "pwm2_enable"), "1", "kept channel untouched");
    }

    #[test]
    fn reload_keeps_failsafe_paths_in_sync() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        let shared = SharedPaths::new(e.enable_paths());
        e.set_failsafe_paths(shared.clone());
        e.take_control().unwrap();

        e.reload(TEST_CONFIG_TWO_CHANNELS).unwrap();
        let pwm1_path = root.path().join("hwmon0/pwm1_enable");
        assert!(shared.snapshot().contains(&pwm1_path));

        e.reload(TEST_CONFIG).unwrap();
        assert!(!shared.snapshot().contains(&pwm1_path));
    }

    #[test]
    fn reload_keeps_legal_overrides_drops_illegal() {
        let root = fake_sysfs();
        let cfg = Config::from_toml_str(TEST_CONFIG_TWO_CHANNELS).unwrap();
        let mut e =
            Engine::from_config(&cfg, TEST_CONFIG_TWO_CHANNELS, root.path(), false).unwrap();
        e.set_override("pwm1", 200, 60).unwrap();
        e.set_override("pwm2", 75, 60).unwrap();
        // New config raises pwm2's floor above the pinned 75 and drops
        // nothing else.
        let stricter = TEST_CONFIG_TWO_CHANNELS.replace("min_pwm = 70", "min_pwm = 90");
        e.reload(&stricter).unwrap();
        assert!(e.overrides.contains_key("pwm1"), "legal override survives");
        assert!(!e.overrides.contains_key("pwm2"), "now-illegal override dropped");
    }

    #[test]
    fn reload_from_disk_follows_config_source() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        let cfg_file = root.path().join("config.toml");
        fs::write(
            &cfg_file,
            TEST_CONFIG.replace("[[40, 80], [70, 200]]", "[[40, 90], [70, 210]]"),
        )
        .unwrap();
        e.set_config_source(cfg_file);
        e.reload_from_disk().unwrap();
        e.tick_once().unwrap();
        // New curve target is 146; ramp starts at the firmware pwm 128 and
        // is limited to one max_step_up of 10.
        assert_eq!(read(&root, "pwm2"), "138");
    }

    #[test]
    fn set_config_applies_and_persists_with_backup() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        let cfg_file = root.path().join("config.toml");
        fs::write(&cfg_file, TEST_CONFIG).unwrap();
        e.set_config_source(cfg_file.clone());

        let hotter = TEST_CONFIG.replace("[[40, 80], [70, 200]]", "[[40, 90], [70, 210]]");
        e.set_config(&hotter).unwrap();

        assert_eq!(fs::read_to_string(&cfg_file).unwrap(), hotter);
        let bak = fs::read_to_string(root.path().join("config.toml.bak")).unwrap();
        assert_eq!(bak, TEST_CONFIG, ".bak holds the previous version");
        assert!(!root.path().join("config.toml.tmp").exists());
    }

    #[test]
    fn set_config_invalid_never_touches_disk() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        let cfg_file = root.path().join("config.toml");
        fs::write(&cfg_file, TEST_CONFIG).unwrap();
        e.set_config_source(cfg_file.clone());

        assert!(e.set_config("not toml at all").is_err());
        assert_eq!(fs::read_to_string(&cfg_file).unwrap(), TEST_CONFIG);
        assert!(!root.path().join("config.toml.bak").exists());
    }

    #[test]
    fn set_config_dry_run_applies_but_never_persists() {
        let root = fake_sysfs();
        let cfg = Config::from_toml_str(TEST_CONFIG).unwrap();
        let mut e = Engine::from_config(&cfg, TEST_CONFIG, root.path(), true).unwrap();
        let cfg_file = root.path().join("config.toml");
        fs::write(&cfg_file, TEST_CONFIG).unwrap();
        e.set_config_source(cfg_file.clone());

        let hotter = TEST_CONFIG.replace("[[40, 80], [70, 200]]", "[[40, 90], [70, 210]]");
        e.set_config(&hotter).unwrap();
        assert_eq!(e.config_toml, hotter, "applied in memory");
        assert_eq!(
            fs::read_to_string(&cfg_file).unwrap(),
            TEST_CONFIG,
            "file untouched in dry-run"
        );
    }

    #[test]
    fn failsafe_drives_full_duty() {
        let root = fake_sysfs();
        let e = engine(&root);
        e.failsafe();
        assert_eq!(read(&root, "pwm2"), "255");
    }

    #[test]
    fn dry_run_never_writes() {
        let root = fake_sysfs();
        let cfg = Config::from_toml_str(TEST_CONFIG).unwrap();
        let mut e = Engine::from_config(&cfg, TEST_CONFIG, root.path(), true).unwrap();
        e.take_control().unwrap();
        e.tick_once().unwrap();
        e.failsafe();
        assert_eq!(read(&root, "pwm2_enable"), "5\n");
        assert_eq!(read(&root, "pwm2"), "128\n");
    }
}
