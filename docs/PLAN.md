# fand — Fan Control Daemon + CLI + GUI for Linux

Project plan / spec for implementation. Target machine and hardware facts below were
**verified by hand on the actual system** — treat them as ground truth, but always
resolve hwmon devices by `name` file at runtime, never by index.

---

## 1. Target system (verified)

- **OS:** CachyOS (Arch-based), kernel 7.1.x-cachyos, Wayland (niri compositor)
- **Shell:** fish (matters for any docs/snippets aimed at the user)
- **CPU:** AMD Ryzen 7 7800X3D → temps via `k10temp` hwmon driver (currently `hwmon4`, do not hardcode)
- **GPU:** NVIDIA RTX 4090, proprietary driver → temps via **NVML** (`nvml-wrapper` crate). GPU's own fans are self-managed (zero-fan mode below ~50–55 °C) — **never control GPU fans**, GPU is a temperature *input* only.
- **Motherboard:** ASRock B650E Taichi, Super I/O = **Nuvoton NCT6799D** (chip ID 0xd802)
  - Kernel driver: `nct6775` (module), exposes hwmon device named `nct6799` (currently `hwmon8`)
  - Driver auto-loads at boot via `/etc/modules-load.d/nct6775.conf` (already configured)
  - No ACPI resource conflict observed; if it ever appears, workaround is `acpi_enforce_resources=lax` (Limine bootloader)

### NCT6799 channel inventory (verified)

| Channel | Wired? | Physical | Notes |
|---|---|---|---|
| fan1 / pwm1 | yes (~636 RPM idle) | **Arctic Liquid Freezer II 360**: pump + VRM fan + 3 rad fans daisy-chained on one header (pump has no tach) | rad has coolant thermal mass → longer smoothing window. **Pump shares this PWM: zero_rpm forbidden; min_pwm ≥ 80** (pump sits at its 800 RPM floor below ~40% duty; firmware auto idles at 77/255, proven safe) |
| fan2 / pwm2 | yes (~775 RPM idle) | Case fans via Lancool 216 built-in controller (PWM passthrough) | verified manual control: pwm=100 → 895 RPM, pwm=255 → 1483 RPM; hub reports one tach for the group |
| fan3–fan5 | no (0 RPM) | empty headers | probe liveness at startup, never write to dead channels |
| pwm7 | exists in sysfs | no matching fan_input | ignore |

- `pwmN_enable` semantics on this chip: `1` = manual, **`5` = firmware auto (verified accepted; this is the restore/failsafe value)**.
- Valid temp sensors on nct6799: `temp1` SYSTIN (~40 °C), `temp2` CPUTIN, `temp7` SMBUSMASTER 0, `temp13` TSI0_TEMP (CPU via TSI, tracks k10temp). AUXTIN0–5 and PCH_* are floating/unwired garbage — must be excluded/flagged as invalid by plausibility checks.

---

## 2. Architecture

One privileged daemon, two unprivileged clients:

```
fand (root, systemd service)   ← ONLY component touching sysfs + NVML; owns control loop + failsafe
  │  Unix socket /run/fand/fand.sock (JSON protocol, newline-delimited)
  ├── fanctl  (CLI, runs as user)
  └── GUI     (Tauri + React, runs as user)
```

### Repo layout (Cargo workspace)

```
fand/
├── Cargo.toml                # workspace
├── crates/
│   ├── fand-core/            # pure logic: curve eval, mix mode, hysteresis, ramping, config types. NO I/O. Heavily unit-tested.
│   ├── fand-proto/           # socket protocol types (serde), shared by daemon + CLI + Tauri backend
│   ├── fand/                 # daemon: sysfs hwmon layer, NVML layer, control loop, failsafe guard, socket server
│   └── fanctl/               # CLI client
├── gui/                      # Tauri 2 app, React + TypeScript frontend
├── systemd/fand.service
└── config/fand.example.toml
```

Key crates: `nvml-wrapper`, `serde` + `toml`, `clap` (fanctl), `tokio` or std threads (daemon is simple enough for threads; tokio fine too), `anyhow`/`thiserror`.

---

## 3. Daemon behavior

### Startup
1. Enumerate `/sys/class/hwmon/*`, resolve devices **by `name`**: need `nct6799` (fans) and `k10temp` (CPU temp). Fail loudly if missing.
2. Init NVML, grab device 0 handle.
3. Probe fan channel liveness: a channel is live iff `fanN_input` > 0 under firmware control (fan1 and fan2 expected). Refuse to control channels not in config AND not live.
4. Record original `pwmN_enable` values, then take manual control (`pwmN_enable = 1`) only for configured channels.

### Control loop (default tick: 2 s, configurable)
```
read cpu_temp   (k10temp Tctl, millidegrees → °C)
read gpu_temp   (NVML)
read board temps as configured (e.g. SYSTIN)
for each configured channel:
    smoothed = rolling average over channel's window (rad channel: longer window, e.g. 10–15 s; case: ~5 s)
    target_pwm = channel policy (see §4)
    apply hysteresis: only move if |target − current| ≥ deadband (e.g. 3 PWM units) or temp crossed a curve point
    ramp: step current toward target, asymmetric — max_step_up per tick (fast, e.g. 10)
          vs max_step_down per tick (slow, e.g. 3) so fans respond to load but decay quietly
    clamp to [channel.min_pwm, 255]; write to sysfs
```

### Plausibility checks (fail loud, fail high)
- Any read error, or temp ≤ 0 °C, or temp ≥ 115 °C ⇒ treat as sensor failure.
- On sensor failure: write **255 to all controlled PWMs**, log, then restore auto mode and exit nonzero (systemd restarts).

### Failsafe (non-negotiable requirements)
- A **guard type** whose `Drop` restores `pwmN_enable = 5` on every controlled channel. Constructed before taking manual control; lives for the daemon's lifetime.
- Signal handlers for SIGTERM/SIGINT trigger clean shutdown through the guard.
- `std::panic::set_hook` (or catch_unwind at top level) also restores auto before aborting.
- systemd `ExecStopPost=/usr/local/bin/fand --restore-auto` covers SIGKILL: a subcommand that just finds nct6799 and writes `pwmN_enable = 5` to all channels, then exits.
- Optional zero-RPM mode for case fans is **opt-in per channel** and requires restart-burst logic: when leaving 0, write a kick duty (~100) for a few seconds before settling to curve value. Default min_pwm floor otherwise ~60–80 so fans can't stall by accident.

---

## 4. Curves and mix mode

FanControl-style model (since phase 7, see REDESIGN.md): a **curve** owns its
temperature source; a **channel** binds exactly one curve by name. Curve kinds:

- **graph**: sorted `(temp_c, pwm)` points evaluated at the curve's own
  sensor; linear interpolation between points, clamped at ends. Also carries
  per-curve input hysteresis (active since phase 8a): the temp feeding the
  curve moves only after departing the accepted value by hysteresis_up/down
  °C for response_seconds, bypassed at curve endpoints by default.
- **mix**: combines other curves' **outputs** with `max` (default) / `min` /
  `average`. → pwm2 (case): max of (cpu_case, gpu_case). Deliberately
  max-of-outputs, NOT one curve fed max-temp — 70 °C means different things
  per component. Cycles are rejected by validation.
- **flat**: constant pwm.

### Example config sketch (TOML)

```toml
[daemon]
tick_seconds = 2

[sensors.cpu]
kind = "hwmon"
hwmon_name = "k10temp"
input = "temp1_input"      # Tctl

[sensors.gpu]
kind = "nvml"
device_index = 0

[curves.cpu_rad]
kind = "graph"
sensor = "cpu"
points = [[40, 80], [60, 130], [75, 200], [85, 255]]

[curves.cpu_case]
kind = "graph"
sensor = "cpu"
points = [[45, 90], [70, 160], [85, 255]]

[curves.gpu_case]
kind = "graph"
sensor = "gpu"
points = [[45, 90], [60, 140], [75, 255]]

[curves.case_mix]
kind = "mix"
function = "max"
curves = ["cpu_case", "gpu_case"]

[channels.pwm1]
hwmon_name = "nct6799"
curve = "cpu_rad"
min_pwm = 80
smoothing_seconds = 12     # radiator thermal mass

[channels.pwm2]
hwmon_name = "nct6799"
curve = "case_mix"
min_pwm = 70
smoothing_seconds = 5
zero_rpm = false           # opt-in; if true, requires kick_pwm + kick_seconds
```

Config lives at `/etc/fand/config.toml`; daemon validates fully before applying (reject unsorted points, pwm out of 0–255, unknown sensor/curve refs, dead channels). Writes from clients are atomic (write temp file + rename) and hot-reloaded.

---

## 5. Socket protocol (fand-proto)

Unix socket `/run/fand/fand.sock`, owner `root:fand`, mode `0660`. User must be in `fand` group (setup script: `groupadd fand; usermod -aG fand $USER`).

Newline-delimited JSON request/response + a subscription stream:

- `get_status` → temps (all sensors), per-channel {rpm, current_pwm, target_pwm, mode}
- `subscribe_status` → server pushes status at 1–2 Hz (feeds live graphs in GUI and `fanctl watch`)
- `get_config` / `set_config` (full validated TOML round-trip)
- `set_override { channel, pwm, ttl_seconds }` → pin a channel temporarily (testing); auto-expires
- `clear_override { channel }`
- Every response: `{ ok: bool, error?: string, data?: ... }`

Version field in every message for forward compat.

---

## 6. fanctl (CLI)

- `fanctl status` — table of temps, RPMs, PWMs
- `fanctl watch` — live updating view (subscribe stream)
- `fanctl curve show/set <name>` — inspect/edit curves
- `fanctl override pwm2 140 --ttl 60`
- `fanctl config edit` (open $EDITOR on a temp copy, validate, apply) / `fanctl config reload`

---

## 7. GUI (Tauri 2 + React + TS)

- Tauri Rust backend reuses `fand-proto` client code; frontend talks to it via Tauri commands/events. Zero privileges needed.
- **Curve editor:** SVG with draggable points per curve (add/remove points, snap, live preview of interpolation). On drag-end → `set_config`.
- **Live dashboard:** rolling line charts of CPU/GPU temps and per-fan RPM/PWM (recharts or d3), fed by `subscribe_status` events.
- **Channel panel:** per-channel policy (single/mix), sensor+curve assignment, min_pwm, smoothing, zero-RPM toggle with kick settings.
- Native Wayland; must work well on niri (no CSD weirdness expected with Tauri, but verify).
- Visual affordance for "override active" and "sensor failure / failsafe engaged" states.

---

## 8. systemd unit

```ini
[Unit]
Description=fand fan control daemon
After=multi-user.target

[Service]
Type=simple
ExecStart=/usr/local/bin/fand
ExecStopPost=/usr/local/bin/fand --restore-auto
Restart=on-failure
RestartSec=2

RuntimeDirectory=fand
RuntimeDirectoryMode=0750
ProtectSystem=strict
ReadWritePaths=/etc/fand
ProtectHome=yes
PrivateTmp=yes
NoNewPrivileges=yes
# NOTE: do NOT set ProtectKernelTunables=yes — it blocks hwmon sysfs writes.
# NVML needs access to /dev/nvidia*; if DevicePolicy is tightened, allow those nodes.

[Install]
WantedBy=multi-user.target
```

---

## 9. Build order (phased, each phase shippable)

1. **fand-core**: config types, curve interpolation, mix-mode eval, hysteresis/ramp state machines. Pure functions + unit tests (test: interpolation endpoints, unsorted-point rejection, max-of-outputs mix, ramp step limits, hysteresis deadband).
2. **Daemon MVP**: hwmon resolve-by-name, NVML read, control loop with hardcoded config, **failsafe guard + panic hook + signal handling**, `--restore-auto` subcommand. Manual test on real hardware (pwm2 first, pwm1 after).
3. **systemd unit + install script** (creates `fand` group, installs binary, enables service).
4. **Socket server + fanctl status/watch** — first end-to-end payoff.
5. **Config file + validation + hot reload + remaining fanctl commands.**
6. **GUI**: dashboard first (read-only), then curve editor, then channel settings.
7. –10. **Redesign** (FanControl curve model + native GNOME GUI): see REDESIGN.md.

## 10. Safety invariants (enforce in code review / tests)

- No PWM write outside 0–255; no write to a channel not both configured and probed-live.
- Every exit path (clean, signal, panic, SIGKILL-via-ExecStopPost) ends with `pwm*_enable = 5`.
- Sensor failure ⇒ 255 everywhere ⇒ restore auto ⇒ exit nonzero. Never loop on stale data.
- min_pwm floor default 60 unless zero_rpm explicitly enabled with kick parameters.
- hwmon devices resolved by `name` every start; indices are not stable across boots.
