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
- **8b — trigger curve + per-channel offset.** ✅ code complete 2026-07-06
  (not deployed — freeze holds). Trigger is a fourth curve kind
  (`CurveConfig::Trigger`): {sensor, idle_temp, idle_pwm, load_temp,
  load_pwm, response_seconds}. `trigger::TriggerLatch` holds {loaded,
  pending_since} — it flips to load at/above load_temp and back to idle
  at/below idle_temp, holding state across the deadband; a crossing must
  persist response_seconds (dwell resets on retreat). First sample latches
  load only if temp ≥ load_temp, else idle (fail-low = quiet until hot; safe
  because triggers can't touch the pump, the min_pwm floor keeps the fan
  spinning, and the ≥115 °C failsafe still escalates). It smooths its sensor
  with the channel window like a graph node, and is a first-class mix
  member. **Forbidden on pwm1**: `Config::validate` walks the pump channel's
  reachable curves (`reaches_trigger`, cycle-safe) and rejects any trigger —
  a step function is wrong for the pump. idle/load pwm below the floor need
  no special case (zero-RPM is gone); the ramp floors every curve kind
  alike. Offset: `offset_pwm: i16` per channel, `apply_offset` adds it to
  the curve output clamped 0..=255 *before* the ramp's min_pwm floor, so the
  floor still wins (a negative offset can't stall a fan). `|offset| ≤ 255`
  validated. Reported target_pwm includes the offset; overrides bypass it.
  GUI surfaces both read-only (editor controls are phase 10).
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

A fourth round (2026-07-10, post-8b review) closed:

- **Non-canonical channel names rejected**: `pwm01` parses to physical
  index 1 but dodged every string check against `PUMP_CHANNEL` (pump floor,
  trigger ban) and could alias `pwm1` as a second TOML key on the same
  header. `is_pwm_name` now requires the name to round-trip through its
  parsed index (`name == format!("pwm{n}")`), so name equality means
  physical-header equality everywhere downstream.
- **GUI read-only surfacing actually delivered**: `offset_pwm` joins
  `ChannelSettings` (shown on the settings card when non-zero), trigger
  payloads carry `response_seconds`, and trigger cards render a full body
  (sensor + live temp, both thresholds/duties, dwell) instead of a bare
  badge.
- **Test gaps**: eval-level proof that trigger inputs share the channel
  smoothing window (a spike above load_temp must not flip the latch);
  engine-level trigger test covering idle/load transitions across ticks,
  deadband hold, status reporting, and latch reset on reload.

## Phase 9 — GUI shell goes native (CSD + adwaita foundation) ✅ code complete 2026-07-17

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

## Phase 10 — Overview redesign (FanControl layout, GNOME skin) ✅ code complete 2026-07-17

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

## Phases 9 + 10 — delivered 2026-07-17

Implemented from the Claude Design project ("fand design system",
`fand.html` interactive mock) rather than from scratch — the mock's
tokens/components were ported into `gui/src/adw/` (Button, Card, Badge,
Banner, Toast, StatusPage, boxed-list rows, Dialog) on Tailwind, with
`index.css` rebuilt on the libadwaita values. Delivered: CSD shell
(`decorations: false`, drag-region headerbar, ⋮ menu, circular close),
single-view Overview (Temperatures chart card, Fans cards, Curves cards +
dashed New-curve), curve editor dialogs for **all four kinds** (graph
two-pane with drag editing + hysteresis-up/down + response rows, batch
Apply via new `update_graph_curve`; mix function/member switches; flat
slider; trigger thresholds), channel-properties dialog (curve, min-duty
floored 60/80, smoothing, offset via new `set_offset_pwm`), New-curve
dialog (graph/mix/flat/trigger via new `create_*` editors), Preferences
(accent only), About, toasts, override banner wired to `ClearOverride`,
disconnect banner + status page with auto-reconnect (verified live).

Deliberate deviations from the mock, all discussed in its own terms:

- Preferences drops the mock's Startup / Safety-tuning / New-channel-
  defaults groups — nothing daemon-side backs them, and the failsafe
  limits are invariants, not settings (a note in the dialog says so).
- Channel properties drops the per-channel "Restore firmware auto" row —
  no such wire command exists (global `--restore-auto` only).
- Empty state drops "Import from config" and the firmware-auto fan cards —
  status only carries configured channels, so undetected headers aren't
  known to the GUI.
- Mix/flat/trigger cards are click-to-edit too (the mock only made graph
  cards activatable); required for "curves page gone with no lost
  capability" (mix membership, deletion) and adds trigger editing.

Post-delivery fixes (2026-07-17, first hands-on feedback): curve-editor
point dragging is now relative (delta from the grab position, mapped
through the SVG's `getScreenCTM` so letterboxing can't offset it — points
no longer jump to the cursor on pickup); Preferences filled out with the
honest option set — Appearance (accent), Overview (chart history 5–30 min,
persisted), Daemon (connection, socket path, working "Reload config from
disk" via `ReloadConfig`), Safety (failsafe 115 °C / floors 60·80 /
restore-on-exit shown read-only as invariants). The mock's Startup/tray/
poll-interval and tunable-failsafe rows stay out: nothing daemon-side
backs them, and the failsafe limits are invariants by design.

Review round 6 (2026-07-18, Anoop's external review agent — all six
findings confirmed and fixed):

- **Config recovery** (blocking): the GUI's curve/settings copies now
  self-heal. The daemon keeps a `config_generation` counter (bumped on
  every successful apply — SetConfig, ReloadConfig, SIGHUP) restated in
  every status frame and returned by GetConfig; the frontend refetches
  when a frame's generation is ahead of its copy, when a fetch failed, or
  after a reconnect (the counter resets with the daemon, so reconnects
  always force a refetch). Level-triggered by design: a missed edge can't
  strand a client, and config edited behind the GUI's back (fanctl,
  SIGHUP) appears within one tick. An unloaded config now shows a
  "waiting for the curve configuration" card, never "No fan curves yet".
- **Flat/trigger round-trip** (blocking): editors keep the raw pwm as
  state and derive the percent display, so open-then-Apply writes back
  exactly the stored values (previously pwm 80 → 31 % → 79).
- **Disconnect wording**: banner + status page now say the state is
  unknown from this window — firmware auto only if the daemon stopped.
- **Estimates marked**: curve-card "duty now" shows ≈ + "estimate" (the
  daemon's hysteresis/dwell/smoothing/floors aren't in that number); the
  temp chart domain auto-expands past 20–100 °C instead of clamping, and
  the graph editor's live marker clamps to the frame edge with the true
  reading instead of vanishing.
- **Keyboard/screen-reader basics**: activatable rows/cards are real
  buttons, dialogs have role/aria-modal/focus trap/focus restore, spin
  steppers and selects are labelled, the header menu has menu semantics
  and Escape-close. (Full SR polish on the SVG editor stays out of scope;
  point editing has a keyboard path via the SpinRows.)
- **Chart timeline**: history is trimmed by sample age (no hardcoded
  2 s tick assumption) and the x axis is real time, so disconnect gaps
  keep their width.

Verified end-to-end against a dry-run daemon: launch-while-down →
recovery, SIGHUP config change appearing untouched, daemon restart with
config edited while down, disconnect wording. 214 workspace tests,
clippy -D warnings (both workspaces), tsc + vite clean.

Review round 7 (2026-07-18, follow-up from the same agent — three fixed,
keyboard graph-point selection explicitly deferred by Anoop):

- **Settings payload versioned**: `get_channel_settings` and the three
  channel-write commands now return
  `{ channels, config_generation }` (mirroring the curve payload), and
  the App staleness check compares both payloads' generations against
  the status stream. Closes the partial-refresh hole: if a generation
  bump refetched both payloads and only the curves fetch succeeded, the
  old map-shaped settings payload had no generation to flag it stale —
  it stayed wrong until the next config change.
- **Temp domains widen everywhere**: `CurveSparkline` and the graph
  editor grow their 20–100 °C base domain (10° steps) to fit any curve
  point — points past 100 °C are legal config and were clipped or
  uneditable. Editor point bounds are now absolute 0–110 °C via the
  spin row; dragging stays clamped to the visible frame so the domain
  can't run away mid-drag. The sparkline's live marker parks at the
  frame edge instead of disappearing when the reading is off-domain.
- **Accessible names**: every Dialog takes a required `label`
  (aria-label on the sheet), Switch takes `ariaLabel` (SwitchRow and
  both mix-member lists pass one), the channel card's curve Select, the
  new-curve name input and the flat editor's slider are labelled.
- **PWM ≠ duty wording**: channel properties' "Min duty" (raw 0–255)
  is now "Min PWM" with a pwm unit and its % duty equivalent in the
  subtitle; "Curve offset" gained the pwm unit.

Verified: dashboard smoke test against a dry-run daemon with a curve
point at 105 °C rendering un-clipped; workspace tests, clippy
-D warnings (both workspaces), fmt, tsc + vite all clean.

Review round 8 (2026-07-18, final review from the same agent — all four
fixed; frontend-only, no Rust changes):

- **Per-hook request sequencing**: `useCurveEditor` and
  `useChannelSettings` number every fetch/write; a response only lands
  in state if no newer request started since (`seq` ref), and both hooks
  expose `invalidate()`, which the App calls on disconnect. Ordinary
  out-of-order responses were already self-corrected by the
  level-triggered generation check within a tick; the real hole was
  across a daemon restart — a response from the *old* daemon carries a
  generation from the old counter, which can sit numerically ahead of
  the restarted daemon's frames, where the `<` staleness check never
  fires and the copy is stuck stale. Invalidate-on-disconnect makes any
  such straggler dead on arrival.
- **Temp cap 110 → 114 °C**: fand-core puts no upper bound on curve
  temps at all; the failsafe fires at ≥ 115 °C, so 114 is the last
  degree a curve can ever act on. The graph editor's `T_HI` and the
  trigger idle/load rows (editor + new-curve dialog) now cap there —
  round 7's 110 was an arbitrary UI clamp of exactly the kind these
  rounds removed.
- **Preferences "Min duty floors" → "Min PWM floors"**: the read-only
  Safety row shows raw pwm 60/80; round 7 fixed the channel dialog but
  missed this one. Subtitle now gives the duty equivalents (24/31 %).
- **Header menu keyboard contract**: `role="menu"` promised behavior it
  didn't have. The popover now moves focus to the first item on open,
  ArrowUp/Down wrap, Home/End jump, Escape closes and returns focus to
  the (now `aria-expanded`-tracked) menu button, Tab closes; items are
  roving `tabIndex={-1}` with a focus-visible highlight.

Verified live against a dry-run daemon: dashboard renders, kill-daemon →
disconnected StatusPage → restart on the same socket → full repopulation
(the exact path the sequencing fix hardens); tsc + vite clean,
`git diff --check` clean.

Round 8 follow-up (2026-07-18, same agent — two issues in the round-8
fixes themselves, both fixed; agent confirms all fan-safety invariants
intact):

- **Straggler sync could disarm reconnect protection**: the seq guard
  stopped stale responses landing in hook state, but a sync in flight
  *across* a disconnect could still settle all-fulfilled and clear
  `forceSync` — with the previously stored old-daemon data (never
  touched by the seq guard) left in place and its high generation
  masking staleness. Two-layer fix: `invalidate()` now also nulls the
  hook's `data` (a null copy re-arms the staleness check by itself,
  whatever else happens), and an App-level `syncEpoch` ref, bumped on
  disconnect, gates the `forceSync.current = false` in the sync
  completion — a sync begun in an older epoch can no longer clear it.
  Side effect, deliberate: a disconnect closes any open editing dialog
  (its data is from a dead daemon whose config may differ on return).
- **Menu focus restoration**: activating a menu item now refocuses the
  menu button *before* running the action — the item is about to
  unmount, and a dialog the action opens snapshots the focused element
  for its close-restore (previously it captured `<body>`). Tab-close
  also refocuses the button first so the browser tabs onward from it.
  The `onMouseLeave` close is gone entirely — focus always sits inside
  the menu now, so hover-out always stranded it — replaced by
  standard outside-`pointerdown` close (GNOME popover behavior).

Verified live: kill/restart cycle again → disconnect page → full
repopulation under the new invalidate/epoch logic; tsc + vite clean.

### GUI data-flow redesign (2026-07-18): backend-owned config

The next review round found two more sync-edge issues (syncs starting
while disconnected + hung requests blocking recovery; dialogs reopening
after reconnect). Rather than a fourth round of patches, Anoop called
the structural question — "the react code [is] a presentation layer not
the full service layer" — and the whole frontend sync machinery was
replaced by backend ownership:

- **`gui/src-tauri/src/state.rs`** now holds the only config copy in the
  GUI process (one `ConfigPayload`: curves + channel bindings + sensors
  + channel settings + generation, so the pieces can never be from
  different generations). The status pump reconciles it on every frame
  (refetches when the frame's generation is ahead), **clears it on every
  new connection** — scoping generation comparisons to one daemon
  lifetime, which deletes the counter-reset problem — and every event
  that carries config is emitted while holding the one cache mutex, so
  the webview receives payloads in exactly cache order.
- Events: `status` now carries `{ status, config }` every tick
  (level-triggered; a failed fetch sends `config: null` and the next
  frame retries); writes emit a `config` event with the daemon-confirmed
  result via `state::publish` (monotonic guard); `daemon-down` repeats
  every 2 s retry instead of firing once — fixing the long-standing
  cosmetic bug where a slow webview missed the one-shot and sat on
  "connecting…".
- **`fand_proto::Client::connect_with_timeout`** (new, unit-tested):
  read/write deadlines on every GUI connection (5 s commands, 15 s
  stream) — a wedged daemon fails a request instead of hanging the pump
  or a command forever. fanctl keeps untimed `connect` (Ctrl-C is the
  user's timeout).
- **Frontend deletions**: both stateful hooks (fetch/cache/seq/
  invalidate), the App self-heal effect and its `syncing`/`forceSync`/
  `syncEpoch` refs — the entire class the last three review rounds mined
  is now unrepresentable. `useDaemonStatus` is the single source
  (status + config + connected); `curves/writes.ts` +
  `settings/writes.ts` are stateless fire-and-report command wrappers.
  Dialogs close on disconnect via one explicit effect (no reopen-after-
  reconnect: the dialog state itself is cleared), and the outside-click
  menu close parks focus on the menu button.

Verified live end-to-end: dashboard renders from pushed events; config
edited on disk + SIGHUP → new curve card appears in the GUI with zero
frontend fetching; kill → disconnect page (config nulled) → restart →
full repopulation including the on-disk change. 215 workspace tests
(incl. the new proto timeout test), clippy -D warnings both workspaces,
fmt, tsc + vite, `git diff --check` all clean. gui/README.md updated to
describe the service-layer/presentation-layer split.

### Backend review round (2026-07-18): instance tokens + async commands

The review agent's first pass over the backend-owned design found the
holes in it — most importantly that clear-on-connect only protected the
*pump's* connection: a write completing against a daemon that restarted
mid-flight could publish its old (huge) generation into the new daemon's
cache, and the `>=` coverage check would then suppress refetching
indefinitely. Fixes, all landed:

- **Daemon instance tokens (protocol change, pre-cutover so free).** The
  daemon draws a random `u64` per process (`random_instance()` in
  engine.rs — std hasher keys, no rand dependency) and stamps it into
  every status frame and `get_config` response; reload keeps it, restart
  changes it. Generations are now ordered *within* an instance and
  **incomparable across instances** — the classic incarnation-number
  fix. `Client::get_config` returns a `ConfigSnapshot { toml,
  generation, instance }`.
- **state.rs guard logic extracted into pure, unit-tested rules**:
  `write_may_advance` (same instance + monotonic; an empty cache refuses
  writes — establishing the current instance is the pump's job),
  `frame_covered` (same instance required, so a poisoned cache self-heals
  on the next frame), `pump_may_install` (pump owns instance changes,
  never steps back within one). Six tests including the exact reviewed
  scenario (stale write gen 50 vs new daemon gen 1 → rejected).
- **Pump refetch moved outside the mutex** — the lock is now held only
  for compare+install+emit (emit-under-lock stays: it *is* the ordering,
  and it's in-process dispatch, not I/O). A slow fetch delays only the
  pump, never a write's publish.
- **All I/O commands are `async`** — Tauri runs sync commands on the
  main thread, so a wedged daemon could previously freeze the window for
  the duration of up to four 5 s socket deadlines. Now it costs a
  background thread a few seconds.
- **Stream deadline derives from the config**: `daemon.tick_seconds` has
  no upper bound, so the fixed 15 s stream timeout would have declared a
  healthy slow-ticking daemon wedged every 15 s (permanent reconnect
  churn). `ConfigPayload` now carries `tick_seconds`; the pump sets the
  stream read timeout to `max(15 s, 3 × tick)` via the new
  `StatusStream::set_read_timeout`.
- **Post-apply read-back failures report honestly**: once `SetConfig`
  succeeded, a failed confirmation read returns "change applied, but
  confirming it failed …" instead of a false failure toast.
- **Config-fetch failures are logged** (were silently swallowed).
- Accepted as designed: a frame may be paired with config one generation
  newer than it was computed under (write raced the tick) — one-frame
  skew, self-correcting, documented at `emit_frame`.

Verified: 222 tests green across both workspaces (216 daemon-side incl.
instance round-trip/persistence tests + 6 GUI guard tests), clippy -D
warnings + fmt clean, tsc + vite clean. Live dry-run: dashboard
populated; kill → disconnect page → restart with `tick_seconds = 20` →
full repopulation under the new instance, and the stream connection's
socket identity was unchanged across 45 s (two >15 s frame gaps) —
no false wedge detection.

### Backend review round 2 (2026-07-18): protocol v2 — CAS + structured outcomes

Second review round on the same day closed the remaining write-race and
outcome-honesty findings, using the last free window for protocol
changes before cutover. **PROTOCOL_VERSION = 2**; there is no v1
interop — daemon, fanctl and GUI ship together at cutover, and a
version mismatch fails with a clear "install together" error on both
ends (request check in the server, response check in the client).

- **Mandatory compare-and-set on SetConfig**: every SetConfig carries
  `expected: ConfigVersion {instance, generation}`; the engine compares
  *before parsing anything*, so a conflict leaves memory, disk,
  generation, overrides and hardware untouched. Every SetConfig caller
  is a read-modify-write that has the pair anyway; the unconditional
  administrative escape hatch is ReloadConfig (edit file, reload).
- **SetConfig outcomes are wire data** (`SetConfigResult`): `Applied
  {toml, version, persistence: Persisted|DryRun}`,
  `AppliedButNotPersisted {toml, version, error}` (a successful
  mutation with a warning — apply-then-persist means the running config
  really changed), `Conflict {current}`, `Rejected {error}`. The
  server's engine-reply timeout carries a structured
  `code: outcome_unknown` (the queued command may still run), and the
  client maps any post-send transport loss to
  `ClientError::OutcomeUnknown` — "may or may not have applied" is now
  typed end to end, never parsed from message text.
- **GUI write gate**: all whole-config RMW commands (and reload)
  serialize through one mutex inside `spawn_blocking` — restoring,
  deliberately this time, the serialization the sync-command era had
  provided by accident (the async conversion in round 1 had removed
  it). CAS backstops against external writers (fanctl).
- **fanctl conflict UX preserves work**: `config edit` sends with the
  fetched version as `expected`; on conflict (or unknown outcome) the
  edited text is copied to `~/.local/state/fanctl/config-edit-<ts>.toml`
  and the message names both versions — user work is never discarded,
  the concurrent change never overwritten. `curve set` does the same
  RMW on one connection.
- **Pump reconciliation tightened**: install a fetched config only when
  its instance equals the triggering frame's; a mismatch proves the
  daemon restarted under the stream → drop the frame and reconnect
  immediately (no daemon-down, no retry delay). A failed fetch never
  touches the cache (a concurrent write may have advanced it); frames
  are emitted with config iff it covers them, else `config: None`; the
  stream timeout derives only from covering config. Fetch-failure
  logging is rate-limited to once per 30 s.
- **publish() returns a typed outcome**; `StaleInstance` (applied to a
  daemon that died before the pump saw the result) surfaces as a
  `write-warning` toast — "applied to a previous daemon… may differ" —
  instead of silent success. `AppliedButNotPersisted` warns the same way.
- **Instance token from /dev/urandom**, redrawn until nonzero (zero is
  reserved for defaulted payloads); an unreadable entropy source fails
  daemon startup (firmware-auto keeps the fans safe) rather than
  degrading to a weak identity.
- **Skew wording corrected everywhere**: config may transiently run
  ahead of queued status frames by more than one generation within the
  same instance — never behind, never across instances.

**The standard this design is measured against** (so future reviews have
a fixed target): (1) safety is entirely the daemon's — no GUI/CLI race
can harm hardware; (2) a user's edit is never silently lost; (3) no
outcome is ever misreported — partial success and unknown outcomes are
said out loud; (4) the display may be up to one tick stale and must
self-heal within one tick.

**Accepted residuals** (documented, reviewer-agreed): unbounded
`UnixStream::connect` (std has no connect timeout for Unix sockets; a
stuck connect needs an alive-but-frozen daemon with a full listen
backlog, and would hold the write gate / pump until the daemon is
restarted — revisit only on field evidence); u64 generation wrap
(~2⁶⁴ mutations in one process lifetime; monotonicity is assumed
within practical lifetimes). Declined: v1/legacy-daemon interop (freeze
+ single cutover; old daemons are also refused syntactically).

Verified: 236 tests green (22 proto incl. CAS wire format + outcome
round-trips + version-mismatch, 72 fand incl. the CAS matrix
[conflict-before-parse proven with garbage TOML], persist-failure,
outcome-unknown reply, nonzero token; 129 core; 5 fanctl incl.
preserve-edit; 8 GUI incl. stale-stream + fetch-failure-preserves-cache),
clippy -D warnings + fmt + tsc/vite clean. Live dry-run, end to end:
`fanctl config edit` with an editor script that raced a concurrent
`curve set` mid-edit → Conflict naming both versions, nothing
overwritten, edit preserved byte-for-byte at the reported path; raw v1
request → clear rejection; GUI populated under v2, kill → disconnect
page → restart → repopulated under the new instance.

### Backend review round 3 (2026-07-19): honesty plumbing + one decline

Follow-up review of the v2 work. One High **declined as designed**, the
rest accepted as small honesty/consistency fixes. Where we drew the
line (Anoop's call, made explicit): *protect the user from the system
lying or eating work invisibly; don't protect the user from their own
visible, deliberate actions.*

- **DECLINED — version-bound GUI editor drafts.** The reviewed race is
  the user editing the same curve in an open GUI dialog and in fanctl
  at once; clicking Apply makes the dialog win. That is WYSIWYG, not a
  silent lost update: the dialog shows exactly what will be applied,
  and Apply re-fetches fresh and surgically edits *only the named
  curve*, so every concurrent change to anything else survives.
  Rejecting Apply on a version mismatch would throw conflict errors at
  the user for their own on-screen edit. Accepted by design.
- The one legitimate sliver of that finding: the fast-restart path
  (StaleStream reconnect) skipped the round-6 rule that a daemon
  restart closes draft dialogs. The pump now emits `daemon-restarted`
  there, and the frontend closes drafts on it exactly as on
  `connected === false`.
- **Warnings ride the invoke result** (reviewer's design, better than
  mine): the `write-warning` event is gone; commands return
  `Ok(Some(warning))`, the frontend's `WriteResult {error, warning}`
  produces exactly one toast per operation — an applied-with-caveat
  warning can no longer be repainted by a racing success toast.
  Instant-apply dialogs (mix members, channel props) show warnings
  inline as "Applied — …" in the dim style, never as red failures.
- **`Client::request_mutating`**: reload/override commands now map
  post-send transport/protocol loss to OutcomeUnknown too (fanctl +
  GUI use it), and a SetConfig ok-response with an uninterpretable
  payload is OutcomeUnknown, not a protocol error. A new
  `ClientError::VersionMismatch` keeps the "install together"
  diagnostic a *known clean refusal* — never blurred into
  outcome-unknown by the mutating path.
- **Server checks a version envelope before deserializing the
  command**, so a v1 SetConfig (which can't even parse as v2) gets
  "unsupported protocol version 1", not "bad request".
- **Preserved edits are now actually never-discarded**: `create_new`
  with a widening suffix (same-second conflicts can't overwrite each
  other), and if copying to the state dir fails, the temp dir's
  auto-delete is disarmed and *its* path reported — the guarantee has
  no failure mode that drops the only copy.

Verified: 241 tests green (25 proto incl. mutating-path OutcomeUnknown
/ version-mismatch-stays-clean / wrong-payload tests, 73 fand incl.
v1-SetConfig diagnostic, 6 fanctl incl. same-second preserve
uniqueness), clippy -D warnings + fmt + tsc/vite clean both
workspaces. Live dry-run: raw v1 set_config → version diagnostic;
override via the mutating path; conflict-preservation re-run; GUI
populated and clean under the new toast plumbing.

### Backend review round 4 (2026-07-19): closing the last honesty gaps

Three findings, all accepted (all "system must not lie / eat work",
none user-protection). The same review independently re-verified all
of round 3 and **closed the declined High**: the WYSIWYG dialog-Apply
trade-off was examined and found to have "no unsafe or untruthful
behavioral outcome".

- **Server executes only newline-terminated requests.** `BufRead`'s
  `read_line` returns an EOF-truncated final record *without* its
  newline, so a client whose `write_all` delivered the full JSON but
  died before the `\n` would report an ordinary send failure ("known
  not-applied") while the daemon parsed and applied the record — a
  one-byte window that broke guarantee 3. The connection loop now
  discards an unterminated final record, making the client's
  send-failure classification provably true (the `request_mutating`
  doc comment now states the server-enforced reason). Proven live
  both ways in a dry-run daemon: a stale round-3 binary applied the
  truncated set_config (generation 0→1 — the bug was real); the fixed
  binary discards it (generation unchanged) and applies the same
  bytes once the newline is present.
- **Engine-reply timeouts are outcome-unknown only for mutations.**
  `forward_to_engine_within` now labels timeouts via `may_mutate`;
  a timed-out GetConfig gets a plain "control loop did not respond in
  time" instead of the OutcomeUnknown code (a read has no outcome to
  be unknown). Reads are listed explicitly so any future command
  defaults to mutating — over-warning is the safe direction.
- **Preserved edits are crash-durable.** `preserve_edit_in` now
  fsyncs the rescue file and its parent directory (the directory sync
  is what makes the new *filename* survive a crash) before returning;
  only then does `keep_edit` drop the TempDir and delete the source.
  Sync failure → the existing keep-the-tempdir fallback.

Verified: 235 root-workspace tests green (75 fand incl.
`eof_truncated_request_is_never_executed` and
`get_config_timeout_is_plain_error_not_outcome_unknown`) + 8 GUI,
clippy -D warnings + fmt clean both workspaces, live dry-run
truncation smoke as above, live service untouched. Frontend
unchanged this round.

Reviewer-accepted residuals: no e2e Tauri→React event test (wiring
inspected, builds clean); no fault-injection test for the
preserve-fallback path (its property — "on failure, delete nothing" —
is structural).

### Backend review round 5 (2026-07-19): durability follow-through

Two findings on the round-4 fixes themselves; both accepted. The
review's verification section confirmed the round-4 framing correct
in full (pipelining, CRLF, clean EOF, `may_mutate` allowlist
"complete and future-safe", no compatibility regression, no
safety-invariant impact).

- **Preserve fsync now covers the whole path chain.**
  `create_dir_all` may create `fanctl/` or deeper ancestors
  (fresh account), and each new directory is itself an entry in
  *its* parent — syncing only the final directory could leave the
  rescue file as an unreachable orphan after a crash. Fix: after
  the file sync, fsync every ancestor of the target directory
  (which levels are new is unknowable; re-syncing a durable
  directory is harmless). Empty component of a relative
  `XDG_STATE_HOME` chain is skipped.
- **The keep-the-tempdir fallback claims no durability it can't
  provide.** `/tmp` is typically tmpfs (RAM) — no fsync survives a
  reboot there, so instead of sync theater the fallback message now
  says the copy is in a temp directory that will not survive a
  reboot and must be copied out now. (Reviewer offered sync-or-say;
  saying is the honest option on tmpfs.)
- **Truncation test made deterministic** (Low): `recv_timeout` only
  proved "nothing arrived yet". Both framing tests now run
  `handle_client` directly on a `UnixStream::pair`, shut down the
  write half, and *join the handler thread* before asserting — once
  it has returned, non-arrival is proof by construction. New
  pipelined test proves the discard is surgical: a complete request
  followed by an unterminated record executes exactly the complete
  one (stub engine records what arrived).

Verified: 236 root-workspace tests green (76 fand), clippy
-D warnings + fmt clean. GUI workspaces untouched this round. The
preserve tests exercise the ancestor fsync walk unprivileged
(incl. `/tmp` and `/`).

### Backend review round 6 (2026-07-19): rescue-path polish

No High/Medium findings; three Lows on the round-5 rescue path, all
accepted. The verification section confirmed round 5 in full
(durability ordering, deterministic framing tests, no protocol or
safety-invariant impact).

- **Sync walk bounded and relative XDG rejected.** The round-5
  every-ancestor walk synced `/`, `/home`, … — wasted, and worse,
  a directory-fsync failure on an unrelated mount (some filesystems
  reject it) would discard an already-durable rescue file. Now the
  deepest *pre-existing* ancestor is found before `create_dir_all`
  and the walk stops there: exactly the target dir plus each newly
  created directory's parent. And per the XDG spec, a relative
  `XDG_STATE_HOME` is invalid and ignored (fall through to
  `~/.local/state`) — extracted as pure `state_base()` with unit
  tests — so the walk only ever sees absolute paths; the round-5
  empty-component filter is gone.
- **No partial rescue artifacts.** A write/sync failure after
  `create_new` succeeded used to leave a truncated
  `config-edit-*.toml` masquerading as a preserved edit (a
  guarantee-3 hazard in file form). The durable write now lives in
  `write_durably()`; on its failure the caller best-effort unlinks
  the partial file (and syncs the directory if the unlink took).
  The temp source still holds the real copy either way.
- **Fallback message claims only what's guaranteed.** Round 5's
  "will not survive a reboot" was itself overconfident — `TMPDIR`
  can be disk-backed (survives reboot) and tmpfiles cleanup can
  purge without one. Now: "not guaranteed to survive cleanup or a
  reboot, copy the file somewhere safe now."

Verified: 237 root-workspace tests green (7 fanctl). All changes in
fanctl; daemon, proto and GUI untouched. Reviewer-noted residual:
power-loss durability and sync-failure cleanup aren't unit-testable
without fault injection — the guard is inspectable by hand.

### Backend review round 7 (2026-07-19): the fallbacks agree on honesty

One Medium, one Low, both on the rescue path, both accepted. The
Medium was not a round-6 regression: the temp-dir last resort dates
from the rescue path's birth — extracting `state_base()` in round 6
(and unit-testing the arm) is what made it reviewable.

- **Medium — no more silent temp-dir "success".** With neither an
  absolute `XDG_STATE_HOME` nor absolute `HOME`, `state_base` fell
  back to the platform temp dir, and a copy there took the ordinary
  success path: presented as a durable rescue while carrying exactly
  the lifecycle risk the keep-the-tempdir fallback warns loudly
  about. The two fallbacks had inverted honesty. Now `state_base`
  returns `Option` (no temp-dir arm) and `preserve_edit` turns
  `None` into an error, which flows into the *existing* honest
  fallback: TempDir retained, "not guaranteed to survive cleanup or
  a reboot" warning, reason attached. Copying one temp file to
  another temp dir improves nothing — fail into the honest path
  instead.
- **Low — cleanup failure is disclosed.** If the best-effort unlink
  of a half-written `config-edit-*.toml` (or the directory sync
  after it) fails, the returned error now says a partial copy may
  remain at that path, with the cleanup error. Previously the
  leftover could later pass for a preserved edit with nothing
  disclosing it.

The verification section confirmed round 6 in full: sync boundary
exact (deepest pre-existing ancestor), symlink handling consistent,
`create_new` forecloses final-file symlink swaps, fallback wording
accurate, no daemon/hardware impact. Verified: 237 root-workspace
tests green; fanctl-only again.

### Backend review round 8 (2026-07-19): no defects; coverage Low declined

"No material functional defects found." One Low: the round-7
fallback/cleanup branches lack direct regression tests (the reviewer
verified them structurally, in the same report). **Declined,
deliberately:** exercising them honestly requires either mutating
process-global env vars (racy across parallel test threads; `set_var`
is `unsafe` in the 2024 edition for this reason) or adding injection
seams / a mock-FS layer to a ~60-line terminal error path whose
design virtue is being short enough to verify by reading. The failure
cost of a regression there is a worse error message, not lost work —
the load-bearing `TempDir::keep()` ownership was verified
structurally this round. First finding where the cure costs more
clarity than the hypothetical disease; revisit only if the rescue
path is changed again. This closes the backend review series.

## What remains

- [x] **Manual click-through** ✅ passed 2026-07-17 (Anoop, dry-run app): all five dialogs,
      point drag/add/remove, window drag + double-click-maximize + close
      on niri, sniff test next to GNOME Settings (phase 9/10 exit criteria).
- [ ] **Commit phases 9–10** (Anoop handles git).
- [ ] **Pre-cutover health check** of the live (old-snapshot) service —
      the ~2026-07-09 burn-in checkup is overdue: `systemctl status fand`,
      `journalctl -u fand` grep FAILSAFE/implausible/restore, memory trend.
- [ ] **THE CUTOVER** (ends the 2026-07-04 deployment freeze, one shot):
      1. migrate /etc/fand/config.toml to the new schema (content should
         match config/fand.example.toml for this machine) **before**
         `sudo scripts/install.sh` — install.sh keeps an existing config
         and runs `fand --check` against it;
      2. `cargo build --release && sudo scripts/install.sh && sudo
         systemctl restart fand`;
      3. verify live: `fanctl status`, GUI against the real socket,
         journal clean, then a fresh burn-in watch.
- [ ] **Phase 8c — `fanctl calibrate <channel>`**: still deferred until
      wanted; explicitly not in the cutover scope.
