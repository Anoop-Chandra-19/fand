//! fand — privileged daemon. The ONLY component that touches sysfs + NVML.
//!
//! Startup: load + validate config, resolve hwmon devices by `name` file
//! (never by index), init NVML if configured, probe channel liveness
//! (fanN_input > 0 under firmware control), install the failsafe (panic
//! hook + Drop guard), then take manual control (pwmN_enable = 1) of
//! configured channels only and run the control loop.
//!
//! Failsafe invariants (plan §3/§10): every exit path — clean, SIGTERM/
//! SIGINT, panic, or SIGKILL via systemd `ExecStopPost=fand --restore-auto`
//! — ends with pwmN_enable = 5 (firmware auto). Sensor failure ⇒ 255 to all
//! controlled PWMs ⇒ restore auto ⇒ exit nonzero.

mod engine;
mod failsafe;
mod hub;
mod hwmon;
mod nvml;
mod server;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::Parser;
use fand_core::Config;
use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

const HWMON_ROOT: &str = "/sys/class/hwmon";
/// The fan chip --restore-auto falls back to when no config is readable
/// (it runs after SIGKILL, so it must not depend on a working config).
const DEFAULT_FAN_CHIP: &str = "nct6799";

#[derive(Parser)]
#[command(name = "fand", version, about = "fan control daemon (runs as root)")]
struct Args {
    /// Config file path
    #[arg(long, default_value = "/etc/fand/config.toml")]
    config: PathBuf,

    /// Validate the config and exit without touching hardware
    #[arg(long)]
    check: bool,

    /// Restore firmware-auto fan control (pwmN_enable = 5) and exit;
    /// used by systemd ExecStopPost to cover SIGKILL
    #[arg(long)]
    restore_auto: bool,

    /// Run the control loop but log decisions instead of writing to hardware
    /// (combine with --socket to test fanctl without root)
    #[arg(long)]
    dry_run: bool,

    /// Unix socket path for fanctl/GUI clients
    #[arg(long, default_value = fand_proto::SOCKET_PATH)]
    socket: PathBuf,
}

fn main() -> ExitCode {
    match run(&Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fand: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<()> {
    if args.restore_auto {
        return restore_auto(&args.config);
    }

    let toml = std::fs::read_to_string(&args.config)
        .with_context(|| format!("reading config {}", args.config.display()))?;
    let cfg = Config::from_toml_str(&toml)
        .with_context(|| format!("loading config {}", args.config.display()))?;
    if args.check {
        println!(
            "config OK: {} sensor(s), {} curve(s), {} channel(s)",
            cfg.sensors.len(),
            cfg.curves.len(),
            cfg.channels.len()
        );
        return Ok(());
    }

    let mut engine = engine::Engine::from_config(&cfg, &toml, Path::new(HWMON_ROOT), args.dry_run)?;

    let shutdown = Arc::new(AtomicBool::new(false));
    for sig in [SIGTERM, SIGINT, SIGQUIT] {
        signal_hook::flag::register(sig, Arc::clone(&shutdown))
            .with_context(|| format!("registering handler for signal {sig}"))?;
    }
    // SIGHUP means "reload the config", the conventional daemon meaning
    // (systemd ExecReload sends it).
    let reload = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGHUP, Arc::clone(&reload))
        .context("registering SIGHUP handler")?;

    // Failsafe in place BEFORE any pwmN_enable is touched. The guard binding
    // must live until run() returns — its Drop is what restores firmware
    // auto. The path list is shared with the engine so hot reloads keep it
    // in sync with the controlled-channel set.
    let _guard = if args.dry_run {
        None
    } else {
        let paths = failsafe::SharedPaths::new(engine.enable_paths());
        failsafe::install_panic_hook(paths.clone());
        engine.set_failsafe_paths(paths.clone());
        Some(failsafe::FailsafeGuard::new(paths))
    };
    engine.set_config_source(args.config.clone());

    let hub = Arc::new(hub::StatusHub::default());
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let (listener, _socket_cleanup) = server::bind(&args.socket)?;
    server::spawn(listener, Arc::clone(&hub), cmd_tx);

    engine.take_control()?;
    engine.run(&shutdown, &reload, &hub, &cmd_rx)
}

/// Standalone cleanup for systemd ExecStopPost: write pwmN_enable = 5 to
/// every pwm channel on the fan chip(s), then exit. Must work even when the
/// config is missing or broken.
fn restore_auto(config_path: &Path) -> Result<()> {
    let mut chip_names = vec![DEFAULT_FAN_CHIP.to_string()];
    if let Ok(toml) = std::fs::read_to_string(config_path) {
        if let Ok(cfg) = Config::from_toml_str(&toml) {
            for ch in cfg.channels.values() {
                if !chip_names.contains(&ch.hwmon_name) {
                    chip_names.push(ch.hwmon_name.clone());
                }
            }
        }
    }

    let mut restored_any = false;
    for name in &chip_names {
        match hwmon::HwmonDevice::find_by_name(Path::new(HWMON_ROOT), name) {
            Ok(dev) => {
                failsafe::restore_all(&dev.all_pwm_enable_paths()?);
                restored_any = true;
            }
            Err(e) => eprintln!("fand: --restore-auto: {e:#}"),
        }
    }
    if restored_any {
        Ok(())
    } else {
        bail!("no fan chip found to restore");
    }
}
