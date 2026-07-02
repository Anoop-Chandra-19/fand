# fand

Fan control daemon + CLI + GUI for Linux (ASRock B650E Taichi / NCT6799D,
AMD k10temp, NVIDIA via NVML). One privileged daemon owns sysfs + NVML and
the failsafe; `fanctl` and a Tauri GUI talk to it over a Unix socket.

Full spec and build phases: [docs/PLAN.md](docs/PLAN.md).
Safety invariants: [CLAUDE.md](CLAUDE.md).

## Status

- [x] Phase 1 — `fand-core`: config types, curve eval, mix mode, hysteresis/ramp (pure + unit tests)
- [x] Phase 2 — daemon MVP: hwmon/NVML, control loop, failsafe guard, `--restore-auto` (code + dry-run verified; live hardware test pending: pwm2 first, then pwm1)
- [ ] Phase 3 — systemd unit + install script
- [ ] Phase 4 — socket server + `fanctl status/watch`
- [ ] Phase 5 — config file, validation, hot reload, remaining fanctl commands
- [ ] Phase 6 — GUI (dashboard → curve editor → channel settings)

## Build

```fish
cargo build --workspace
cargo test --workspace
```
