---
name: verify
description: Build, launch and observe fand's daemon/CLI/GUI without touching real hardware
---

# Verifying fand changes (no root, no hardware writes)

The live systemd service owns the real fans — never run a repo build
against hardware. Everything below is unprivileged and read-only.

## One-command GUI sessions

- `make dev` — GUI + repo-built `fand --dry-run` on a temp socket (real
  sensors, no writes). Ctrl-C tears everything down.
- `make dev-mock` — GUI + `crates/fand/examples/mockd.rs`, a fake daemon
  speaking full protocol v2 with synthetic temps run through the real
  fand-core curve trees. `SCENARIO=heat-ramp` sweeps 35→92 °C (full curve
  range), `SCENARIO=flappy` drops clients every 20 s (disconnect UI),
  `SCENARIO=restart` also comes back as a new instance (generation reset).
- Mock alone for CLI work:
  `cargo run -p fand --example mockd -- --socket /tmp/mockd.sock`

## Daemon + CLI surface

```fish
set S (mktemp -d)
cp config/fand.example.toml $S/config.toml   # SetConfig persists to --config; never point at the repo file
cargo build -p fand -p fanctl
target/debug/fand --dry-run --config $S/config.toml --socket $S/fand.sock &
target/debug/fanctl --socket $S/fand.sock status
```

Dry-run reads the real sensors (k10temp/nct6799/NVML work unprivileged)
and logs PWM decisions instead of writing them, so status shows real
temps/RPM.

## GUI surface (Tauri window on the live niri session)

```fish
cd gui; FAND_SOCKET=$S/fand.sock npm run tauri dev
```

- Window appears on the user's desktop (App ID `fand-gui`); wait for it:
  `niri msg windows | grep fand-gui`.
- Screenshot: `niri msg action focus-window --id <id>`, then
  `niri msg action screenshot-window`, then read it from the clipboard:
  `wl-paste --type image/png > shot.png` (nothing is written to
  ~/Pictures; the clipboard is the only output).
- No input automation is installed (no ydotool/wtype) — dialog
  click-throughs are manual; kill/restart the dry-run daemon to drive the
  disconnect/reconnect states instead.
- Frontend-only check: `npm run build` in gui/ (tsc + vite).
