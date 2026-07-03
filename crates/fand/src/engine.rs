//! Startup wiring + the control loop. Per channel each tick:
//!
//! ```text
//! sensor temps → RollingAverage → Curve::eval / mix::eval_max → Ramp::step → pwm write
//! ```
//!
//! Any tick error (sensor read failure, implausible reading, failed write)
//! drives every controlled fan to full and exits nonzero — never loop on
//! stale data.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use fand_core::config::{Policy, SensorConfig};
use fand_core::{mix, window_ticks, Config, Curve, Kick, Ramp, RampConfig, RollingAverage};
use fand_proto::{ChannelStatus, Status};

use crate::hub::StatusHub;
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

enum SensorSource {
    Hwmon { hwmon_name: String, input: String },
    Nvml { device_index: u32 },
}

struct ChannelInput {
    sensor: String,
    curve: Curve,
    smoother: RollingAverage,
}

struct Channel {
    name: String,
    hwmon_name: String,
    pwm_index: u32,
    inputs: Vec<ChannelInput>,
    ramp: Ramp,
    last_written: Option<u8>,
}

pub struct Engine {
    devices: BTreeMap<String, HwmonDevice>,
    gpu: Option<Gpu>,
    sensors: BTreeMap<String, SensorSource>,
    channels: Vec<Channel>,
    tick: Duration,
    dry_run: bool,
}

impl Engine {
    /// Resolve hardware and build per-channel state from a **validated**
    /// config. Fails loudly on: missing hwmon chip, NVML init failure, or a
    /// configured channel whose fan shows no tach under firmware control.
    pub fn from_config(cfg: &Config, hwmon_root: &Path, dry_run: bool) -> Result<Self> {
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
            if rpm == 0 {
                bail!(
                    "channel `{name}`: fan{pwm_index}_input reads 0 RPM under firmware \
                     control — dead header, refusing to control it"
                );
            }

            let policy_inputs: Vec<(&String, &String)> = match &ch.policy {
                Policy::Single { sensor, curve } => vec![(sensor, curve)],
                Policy::Mix { inputs } => {
                    inputs.iter().map(|i| (&i.sensor, &i.curve)).collect()
                }
            };
            let window = window_ticks(ch.smoothing_seconds, tick_seconds);
            let mut inputs = Vec::new();
            for (sensor, curve_name) in policy_inputs {
                let curve_cfg = cfg
                    .curves
                    .get(curve_name)
                    .with_context(|| format!("channel `{name}`: unknown curve `{curve_name}`"))?;
                inputs.push(ChannelInput {
                    sensor: sensor.clone(),
                    curve: Curve::try_from(curve_cfg)
                        .with_context(|| format!("curve `{curve_name}`"))?,
                    smoother: RollingAverage::new(window),
                });
            }

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
                inputs,
                ramp,
                last_written: None,
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
            tick: Duration::from_secs(tick_seconds),
            dry_run,
        })
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

    /// Tick until `shutdown` is set (SIGTERM/SIGINT), publishing a status
    /// snapshot to `hub` each tick. On any tick error: fans to full, then
    /// propagate — the caller's guard restores auto.
    pub fn run(&mut self, shutdown: &AtomicBool, hub: &StatusHub) -> Result<()> {
        while !shutdown.load(Ordering::Relaxed) {
            match self.tick_once() {
                Ok(status) => hub.publish(status),
                Err(e) => {
                    self.failsafe();
                    return Err(e.context(
                        "control tick failed — drove fans to full; firmware auto restored on exit",
                    ));
                }
            }
            sleep_interruptible(self.tick, shutdown);
        }
        eprintln!("fand: shutdown requested");
        Ok(())
    }

    fn tick_once(&mut self) -> Result<Status> {
        let temps = read_temps(&self.sensors, &self.devices, self.gpu.as_ref())?;
        let mut channel_status = BTreeMap::new();
        for ch in &mut self.channels {
            let mut evals = Vec::with_capacity(ch.inputs.len());
            for input in &mut ch.inputs {
                let temp = *temps
                    .get(&input.sensor)
                    .with_context(|| format!("channel `{}`: sensor `{}`", ch.name, input.sensor))?;
                evals.push((input.smoother.push(temp), &input.curve));
            }
            let raw_target = mix::eval_max(&evals)
                .with_context(|| format!("channel `{}` has no inputs", ch.name))?;
            let pwm = ch.ramp.step(raw_target);

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
                    mode: "manual".to_string(),
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

/// Sleep in short slices so SIGTERM/SIGINT interrupt a tick wait promptly.
fn sleep_interruptible(total: Duration, shutdown: &AtomicBool) {
    const SLICE: Duration = Duration::from_millis(200);
    let mut remaining = total;
    while !shutdown.load(Ordering::Relaxed) && remaining > Duration::ZERO {
        let slice = remaining.min(SLICE);
        thread::sleep(slice);
        remaining = remaining.saturating_sub(slice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failsafe::FailsafeGuard;
    use std::fs;

    /// Fake /sys/class/hwmon with an nct6799 (fan2 live) and a k10temp.
    fn fake_sysfs() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let nct = dir.path().join("hwmon0");
        fs::create_dir(&nct).unwrap();
        fs::write(nct.join("name"), "nct6799\n").unwrap();
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
        points = [[40, 80], [70, 200]]

        [channels.pwm2]
        hwmon_name = "nct6799"
        policy = "single"
        sensor = "cpu"
        curve = "c"
        min_pwm = 70
        smoothing_seconds = 2
    "#;

    fn engine(root: &tempfile::TempDir) -> Engine {
        let cfg = Config::from_toml_str(TEST_CONFIG).unwrap();
        Engine::from_config(&cfg, root.path(), false).unwrap()
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
        let Err(err) = Engine::from_config(&cfg, root.path(), false) else {
            panic!("dead channel must be refused");
        };
        assert!(err.to_string().contains("dead header"), "{err:#}");
    }

    #[test]
    fn missing_chip_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let cfg = Config::from_toml_str(TEST_CONFIG).unwrap();
        assert!(Engine::from_config(&cfg, root.path(), false).is_err());
    }

    #[test]
    fn take_control_writes_manual_and_guard_restores_auto() {
        let root = fake_sysfs();
        let e = engine(&root);
        e.take_control().unwrap();
        assert_eq!(read(&root, "pwm2_enable"), "1");

        drop(FailsafeGuard::new(e.enable_paths()));
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
    fn tick_reports_status() {
        let root = fake_sysfs();
        let mut e = engine(&root);
        let status = e.tick_once().unwrap();
        assert_eq!(status.temps["cpu"], 54.0);
        let ch = &status.channels["pwm2"];
        assert_eq!(ch.rpm, 789);
        assert_eq!(ch.current_pwm, 136);
        assert_eq!(ch.target_pwm, 136);
        assert_eq!(ch.mode, "manual");
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
        let mut e = Engine::from_config(&cfg, root.path(), true).unwrap();
        e.take_control().unwrap();
        e.tick_once().unwrap();
        e.failsafe();
        assert_eq!(read(&root, "pwm2_enable"), "5\n");
        assert_eq!(read(&root, "pwm2"), "128\n");
    }
}
