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
- min_pwm floor default 60+ unless zero_rpm is explicitly enabled with kick
  parameters (kick_pwm ~100 for a few seconds when leaving 0).
- Resolve hwmon devices by `name` file at every start ("nct6799", "k10temp");
  indices are not stable across boots.
- Never control GPU fans — the GPU is a temperature input only (via NVML).
- Mix mode is max-of-outputs (each curve evaluated at its own sensor), never
  one curve fed the max temperature.

## Workflow

- `fand-core` stays pure (no I/O) and heavily unit-tested; all sysfs/NVML
  access lives in the `fand` crate.
- Manual hardware tests: pwm2 (case) first, pwm1 (CPU radiator) after.
- Build order is phased (plan §9); each phase should be shippable.
- User's shell is fish — write any user-facing snippets/docs accordingly.
