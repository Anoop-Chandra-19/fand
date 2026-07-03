# fand

Fan control daemon + CLI + GUI for Linux (ASRock B650E Taichi / NCT6799D,
AMD k10temp, NVIDIA via NVML). One privileged daemon owns sysfs + NVML and
the failsafe; `fanctl` and a Tauri GUI talk to it over a Unix socket.

Full spec and build phases: [docs/PLAN.md](docs/PLAN.md).
Safety invariants: [CLAUDE.md](CLAUDE.md).

## Status

- [x] Phase 1 — `fand-core`: config types, curve eval, mix mode, hysteresis/ramp (pure + unit tests)
- [x] Phase 2 — daemon MVP: hwmon/NVML, control loop, failsafe guard, `--restore-auto` (live-tested on real hardware: pwm2 solo, then pwm1+pwm2 under load, clean restore both times)
- [x] Phase 3 — systemd unit + install script (`scripts/install.sh`; enabling the service stays manual)
- [x] Phase 4 — socket server (`get_status`/`subscribe_status`) + `fanctl status/watch`
- [ ] Phase 5 — config file, validation, hot reload, remaining fanctl commands
- [ ] Phase 6 — GUI (dashboard → curve editor → channel settings)

## Build

```fish
cargo build --workspace
cargo test --workspace
```

## Install

```fish
cargo build --release
sudo scripts/install.sh
```

Installs `fand`/`fanctl` to /usr/local/bin, the systemd unit, and a default
`/etc/fand/config.toml` (never overwrites an existing one); creates the
`fand` group and adds you to it (re-login to pick it up). It does **not**
enable the service — after the manual hardware test has passed:

```fish
sudo systemctl enable --now fand
```

Emergency hand-back to firmware at any time:

```fish
sudo fand --restore-auto
```

Uninstall (keeps `/etc/fand/`): `sudo scripts/install.sh uninstall`
