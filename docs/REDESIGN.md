# Redesign plan — FanControl model + native GNOME GUI

Continues the build order in PLAN.md §9 (phases 1–6 shipped). Research and
rationale: RESEARCH.md. Same rules as always: each phase shippable, phases
start on explicit go-ahead, all PLAN.md §10 safety invariants hold throughout.

The two tracks are independent after phase 7: 8 (daemon behaviors) and 9–10
(GUI) can be reordered if we want visible progress first.

---

## Target config schema (agreed shape, phase 7)

Temperature source and mixing move from the channel into the curve; a channel
always binds exactly one curve.

```toml
[sensors.cpu]
kind = "hwmon"
hwmon_name = "k10temp"
input = "temp1_input"

[curves.cpu_rad]                    # "graph" — points + own sensor
kind = "graph"
sensor = "cpu"
points = [[40, 80], [60, 130], [75, 200], [85, 255]]
hysteresis_up = 2.0                 # °C, input-side (active since phase 8a)
hysteresis_down = 3.0
response_seconds = 1
ignore_hysteresis_at_extremes = true

[curves.case_mix]                   # "mix" — combines other curves' outputs
kind = "mix"
function = "max"                    # max | min | average; max is the default
curves = ["cpu_case", "gpu_case"]

[curves.bench]                      # "flat" — constant duty
kind = "flat"
pwm = 128

[channels.pwm2]
hwmon_name = "nct6799"
curve = "case_mix"                  # one curve, always
min_pwm = 70
smoothing_seconds = 5
# max_step_up / max_step_down / deadband / zero_rpm / kick_* unchanged
# offset_pwm = 0                    # phase 8b, per-channel bias
```

Validation additions: unknown curve/sensor refs anywhere in the tree; mix
cycles (walk the graph, reject any curve reachable from itself); empty mix;
flat pwm 0–255; hysteresis/response values sane (≥ 0, hysteresis < 20 °C).

Migration: no back-compat parsing. `/etc/fand/config.toml` is two channels —
hand-migrate it when phase 7 ships, before `systemctl restart fand`.
`config/fand.example.toml` and all test fixtures updated in the same change.

---

## Phase 7 — Mix becomes a curve type (schema restructure) ✅ code complete 2026-07-04

Behavior-identical restructure: same PWM outputs as today, new model.
Shipped in code; parity verified side-by-side in dry-run (old binary + old
schema vs new binary + new schema: identical duties/targets).

**Deliberately not deployed** — decision 2026-07-04: the live service stays
on the pre-7 snapshot until the full redesign (through phase 10) ships as
one cutover. Until then all repo-built config/curve tooling runs against a
dry-run daemon only; the live socket's config text is old-schema and the
new `fanctl curve`/GUI config paths will (correctly) refuse it. Cutover
steps live in this file's phase-7 exit criteria + install.sh notes: swap
`/etc/fand/config.toml` for the migrated config **before** running
install.sh (its `--check` runs against the kept config).

- `fand-core/config.rs`: `CurveConfig` → tagged enum (graph / mix / flat);
  `Policy` and `MixInput` deleted; channel gets `curve: String`. New
  validation per above (cycle check is the only new algorithm).
- `fand-core`: curve-tree evaluation — resolve a channel's curve, evaluate
  recursively (graph → interp at its sensor's temp; mix → function over
  members; flat → constant). `mix.rs`'s max-of-outputs becomes the `max` arm;
  add `min`/`average`. Hysteresis fields parsed + validated but **inert**
  until 8a.
- `fand-core/policy_edit.rs` → folded into `curve_edit.rs` (create/delete/
  retarget curves, edit mix membership); `channel_edit.rs` shrinks to
  hardware params + curve binding.
- `fand/engine.rs`: `build()` resolves curve trees (sensors referenced
  anywhere in a channel's tree get read each tick); `tick_once()` calls tree
  eval. Smoothing, deadband, ramp, min_pwm, zero-rpm, overrides untouched.
- `fand-proto` + Tauri commands: curve payloads carry kind/source/members;
  `fanctl curve show/set` updated.
- Tests: port existing engine/config tests to new schema; new: cycle
  rejection, min/average mix, flat curve, mix-of-mix evaluates correctly,
  channel-binds-missing-curve rejected.

**Exit criteria:** dry-run daemon produces identical PWM trace to pre-7 build
on the same temps; live config migrated; service restarted clean.

Safety note: max stays the documented default mix function; `average`/`min`
never silently replace it — GUI and fanctl must show the function explicitly
(a min-mix on pwm2 is how you cook a GPU quietly).

## Phase 8 — FanControl behaviors

- **8a — input hysteresis + response time** (the flagship). ✅ code complete
  2026-07-04 (not deployed — freeze holds). `fand-core/hysteresis.rs`:
  `InputFilter` sits between each graph node's smoother and interpolation,
  holding {accepted temp, pending excursion (direction, since)}; the curve's
  input moves only after departing ≥ hysteresis_up/down °C for ≥
  response_seconds (dwell restarts if the temp retreats into the band or
  flips direction); bypassed at/beyond the curve's endpoint temps when
  `ignore_hysteresis_at_extremes` (default true — a spike past the last
  point hits full duty immediately). All-default knobs build no filter at
  all, so existing configs behave identically. State lives in the per-
  channel `CurveTree` and resets on reload. `CurveTree::eval` now takes the
  tick's `Instant`. Revisit per-channel `smoothing_seconds`/`deadband`
  defaults once hysteresis proves out on pwm2 — likely lower smoothing,
  keep deadband.
  Tests (all in): no output change within band; change accepted after dwell;
  down slower than up; extremes bypass (and honored when disabled); dwell
  reset on retreat/direction flip; reload resets; engine-level wiring.
- **8b — trigger curve + per-channel offset.** Trigger: {idle_temp, idle_pwm,
  load_temp, load_pwm, response_seconds}, latches between thresholds.
  **Forbidden on pwm1** (same validation class as zero_rpm); idle_pwm below
  MIN_PWM_FLOOR requires the channel's zero_rpm opt-in. Offset: signed add
  post-curve, pre-clamp — clamp order stays min_pwm..255.
- **8c — `fanctl calibrate <channel>`** (deferred until wanted). Interactive
  sweep to find real stop/start duties to *suggest* min_pwm/kick values.
  Hard gates: refuses pwm1 outright (pump inline); requires `--i-know`
  confirmation; overrides expire via existing TTL mechanism; restores curve
  mode on any error/^C. Never runs unattended.

## Phase 9 — GUI shell goes native (CSD + adwaita foundation)

- `tauri.conf.json`: `decorations: false`; title stays "fand" (alt-tab only).
- Headerbar component: `data-tauri-drag-region`, circular close button
  (GNOME default layout = close only), ⋮ primary menu → Preferences, About.
- Sidebar deleted (Overview absorbs Curves in phase 10; Settings becomes the
  Preferences dialog).
- `index.css` rebuilt on libadwaita values (RESEARCH.md §B.2–B.5): palette as
  `--window-bg-color`-style custom properties, Adwaita Sans/Mono stack,
  typography classes, radii (9/12/15), 6px spacing rhythm, `tabular-nums` on
  every live readout.
- Verify drag/double-click-maximize and close on niri; tiled = square
  corners, no transparency work.

**Exit criteria:** window is one headerbar + content, draggable, closes;
"fand" appears at most once on screen.

## Phase 10 — Overview redesign (FanControl layout, GNOME skin)

- Single main view: **Controls** section (per-channel cards: name, curve
  dropdown, live % + RPM in `.numeric`, ⋮ → channel properties dialog) and
  **Curves** section (cards with sparkline preview, kind badge, live source
  temp + output; click → editor).
- Curve editor dialog: Cancel / suggested-action Apply in dialog header
  (batch apply — no half-edited curves reach hardware); graph for graph
  curves, member list + function for mix, single slider for flat; hysteresis
  controls once 8a ships.
- Channel properties dialog: boxed-list rows, instant apply through existing
  validation; zero-RPM as expander row (locked with explanatory subtitle on
  pwm1); min_pwm spin row hard-floored at 60/80 per invariants.
- Preferences dialog (app-level only): tick interval, appearance. About
  dialog.
- Feedback: toast on apply; warning banner while an override is active or
  socket drops; status page when daemon unreachable.

**Exit criteria:** curves page gone with no lost capability; every daemon
state (override, failsafe, disconnect) visibly surfaced; side-by-side sniff
test against a real libadwaita app (GNOME Settings) passes.
