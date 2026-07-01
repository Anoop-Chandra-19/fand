//! fand — privileged daemon. The ONLY component that touches sysfs + NVML.
//!
//! Startup: resolve hwmon devices by `name` file (need "nct6799" and
//! "k10temp" — never by index), init NVML, probe channel liveness
//! (fanN_input > 0 under firmware control), record original pwmN_enable,
//! then take manual control (pwmN_enable = 1) of configured channels only.
//!
//! Failsafe invariants (see plan §3/§10):
//! - Guard type whose Drop restores pwmN_enable = 5 on every controlled
//!   channel; constructed before taking manual control.
//! - SIGTERM/SIGINT → clean shutdown through the guard; panic hook restores
//!   auto before aborting.
//! - `fand --restore-auto` subcommand (used by systemd ExecStopPost to cover
//!   SIGKILL): find nct6799, write pwmN_enable = 5 to all channels, exit.
//! - Sensor failure (read error, temp ≤ 0 °C or ≥ 115 °C) ⇒ 255 to all
//!   controlled PWMs, restore auto, exit nonzero.

fn main() {
    // TODO phase 2: clap args (--restore-auto), hwmon layer, NVML layer,
    // control loop, failsafe guard.
    eprintln!("fand: not implemented yet (phase 2)");
    std::process::exit(1);
}
