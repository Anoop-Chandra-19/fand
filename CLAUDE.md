# fand — fan control daemon + CLI + GUI for Linux

Full spec: `docs/PLAN.md` (hardware facts there were verified by hand on the
target machine — treat as ground truth). Cargo workspace: `fand-core` (pure
logic, no I/O), `fand-proto` (socket types), `fand` (root daemon), `fanctl`
(CLI), `gui/` (Tauri 2, phase 6).

## Safety invariants — enforce in every change and review

This daemon writes to real fan hardware as root. Non-negotiable:

- No PWM write outside 0–255; never write to a channel that is not both
  configured and probed-live (`fanN_input > 0` under firmware control).
- Every exit path — clean, signal, panic, SIGKILL-via-ExecStopPost — must end
  with `pwm*_enable = 5` (firmware auto; verified restore value on NCT6799).
- Sensor failure (read error, temp ≤ 0 °C, temp ≥ 115 °C) ⇒ write 255 to all
  controlled PWMs ⇒ restore auto ⇒ exit nonzero. Never loop on stale data.
- min_pwm floor 60 on every channel; fans never stop (zero-RPM mode was
  removed 2026-07-06 — the GPU's own driver handles GPU fan idle-stop, and
  fand never controls GPU fans anyway).
- **pwm1 carries the AIO pump inline** (Arctic Liquid Freezer II 360: pump +
  VRM fan + 3 rad fans on one header, pump has no tach). Its min_pwm stays
  ≥ 80 (~31% duty — at/above the firmware-auto idle of 77/255 this system
  has proven safe). Both floors are enforced in `Config::validate`.
- Resolve hwmon devices by `name` file at every start ("nct6799", "k10temp");
  indices are not stable across boots.
- Never control GPU fans — the GPU is a temperature input only (via NVML).
- Mixes combine member curve *outputs* (each curve evaluated at its own
  sensor), never one curve fed a combined temperature. `max` is the default
  and the safety-documented choice; `min`/`average` are explicit opt-ins
  (phase-7 decision) that clients must always display — a min-mix can
  under-cool the hotter component.

## Workflow

- `fand-core` stays pure (no I/O) and heavily unit-tested; all sysfs/NVML
  access lives in the `fand` crate.
- Manual hardware tests: pwm2 (case) first, pwm1 (CPU radiator) after.
- Build order is phased (plan §9); each phase should be shippable. Phases
  start on the user's explicit go-ahead, not automatically.
- The user manages git himself — never commit or push unless asked.
- The user is learning Rust with this project: explain concepts in chat
  when walking through new code; code comments only for genuinely subtle
  logic, never for teaching syntax.
- User's shell is fish — write any user-facing snippets/docs accordingly.

## Deployed service (since 2026-07-02)

fand is installed system-wide and enabled: `fand.service` controls the real
fans at boot from `/usr/local/bin/fand` (a snapshot — NOT the repo build),
config at `/etc/fand/config.toml`.

- Never run a repo-built daemon against real hardware while the service is
  up; `sudo systemctl stop fand` first (its ExecStopPost restores firmware
  auto). Develop unprivileged instead:
  `fand --dry-run --socket /tmp/... ` + `fanctl --socket /tmp/...`.
- Shipping a change: `cargo build --release && sudo scripts/install.sh &&
  sudo systemctl restart fand` (the user runs privileged commands himself).
- **DEPLOYMENT FREEZE (2026-07-04):** the service runs a pre-redesign
  snapshot with old-schema config; nothing ships until redesign phases
  7–10 (docs/REDESIGN.md) are all done, then one cutover — swap
  /etc/fand/config.toml for the migrated config *before* install.sh (its
  `--check` runs against the kept config). Repo-built `fanctl curve`/GUI
  config paths refuse the live daemon's old-schema config text — develop
  against a dry-run daemon only.
- Emergency hand-back at any time: `sudo fand --restore-auto`.
- Health checkup: `systemctl status fand` (uptime/restarts/memory trend),
  `journalctl -u fand` (grep FAILSAFE / implausible / restore),
  `sg fand -c "fanctl status"`.
