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
# max_step_up / max_step_down / deadband unchanged
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
  **Forbidden on pwm1** (same validation class as the pump floor); idle_pwm
  may never go below MIN_PWM_FLOOR — zero-RPM mode no longer exists (see the
  hardening pass below), fans never stop. Offset: signed add post-curve,
  pre-clamp — clamp order stays min_pwm..255.
- **8c — `fanctl calibrate <channel>`** (deferred until wanted). Interactive
  sweep to find real stall/start duties to *suggest* min_pwm values.
  Hard gates: refuses pwm1 outright (pump inline); requires `--i-know`
  confirmation; overrides expire via existing TTL mechanism; restores curve
  mode on any error/^C. Never runs unattended.

### Post-8a hardening (2026-07-06, external review findings) ✅ code complete

Applied after a code review of the phase-7/8a work; all in the cutover scope:

- **Zero-RPM mode removed outright** (Anoop's decision): it existed to mimic
  NVIDIA's idle fan stop, but the GPU's driver already does that and fand
  never controls GPU fans — motherboard-header fans should always run.
  `zero_rpm`/`kick_pwm`/`kick_seconds` are gone from schema, ramp, engine,
  channel_edit, GUI; an old config carrying them fails at parse.
- **pwm1 pump floor is daemon-enforced**: `Config::validate` rejects
  min_pwm < 80 on pwm1 (`PUMP_CHANNEL`/`PUMP_MIN_PWM_FLOOR` in fand-core),
  closing the fanctl/hand-edit/SIGHUP bypass around the old GUI-only clamp.
- **Reload can no longer orphan a dropped channel**: if restoring firmware
  auto on a channel the new config drops fails, it is tracked as an
  `UnrestoredChannel` (name + pwm/enable paths) whose enable path stays on
  the failsafe guard's restore list — so the "every exit restores auto"
  invariant holds even through failed hand-backs. (The third round below
  hardens this further: immediate 255 park, reload-time retry, re-add
  cleanup.)
- **Only tree-referenced sensors are read**: engine build resolves curve
  trees first and initializes/reads exactly the sensors they reference
  (`CurveTree::sensors()`), so an unused sensor can't block startup or
  trip the failsafe.
- `deny_unknown_fields` on Config/ChannelConfig; stale `target_pwm` doc
  comment in fand-proto fixed.

A second review round (same day) added:

- **Ramp floor bug (pre-existing, critical):** the deadband check could hold
  a pwm *below* min_pwm forever (firmware hands over at 78, pwm1 floor 80,
  deadband 3 ⇒ stuck at 78). The deadband now never applies while current
  is under the floor.
- **Hot reload refuses hardware re-binding:** keeping `[channels.pwm2]` but
  changing its `hwmon_name` is a different fan — it would skip the liveness
  probe and strand the old header in manual mode. Reload bails ("restart
  fand instead") before touching hardware.
- **Aborted-reload rollback failures are tracked:** if switching a batch of
  new channels fails partway, rollback restores of already-switched ones
  can themselves fail — those now join `unrestored` (name + pwm path +
  enable path) like failed drops.
- **failsafe() covers unrestored channels:** stuck-manual channels get
  driven to 255 alongside live ones (they're at our last-written duty
  otherwise, and the exit restore might fail again).
- `SensorConfig` restructured to newtype variants (like `CurveConfig`) so
  its payload structs carry `deny_unknown_fields` too; parse-rejection
  tests prove the attribute fires through the internally-tagged enum.
- CLAUDE.md mix invariant reworded to match phase 7: the invariant is
  *outputs-not-temperatures*; max is default, min/average are explicit
  opt-ins clients must display.

A third round refined the `unrestored` machinery itself:

- **Stuck channels are parked at 255 immediately** (`park_unrestored`) when
  a hand-back fails, not only on a later failsafe — their last curve duty
  could be idle-quiet and nobody is driving them anymore.
- **Every reload retries the hand-back** of carried unrestored entries and
  drops the ones that succeed — the daemon self-heals instead of waiting
  for process exit.
- **Re-adding a stuck channel reclaims it**: the add path removes matching
  entries, which the retry made mandatory (a stale entry would otherwise
  hand a just-reclaimed channel back to firmware mid-reload).
- Both failure branches (dropped-channel hand-back, aborted-reload
  rollback) now share `park_unrestored`, so the rollback branch — whose
  exact I/O failure sequence can't be forced in tests — runs the same code
  the drop-path tests exercise.

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
  validation; min_pwm spin row hard-floored at 60/80 per invariants (no
  zero-RPM controls — the mode was removed in the post-8a hardening pass).
- Preferences dialog (app-level only): tick interval, appearance. About
  dialog.
- Feedback: toast on apply; warning banner while an override is active or
  socket drops; status page when daemon unreachable.

**Exit criteria:** curves page gone with no lost capability; every daemon
state (override, failsafe, disconnect) visibly surfaced; side-by-side sniff
test against a real libadwaita app (GNOME Settings) passes.
