# Design research — FanControl parity + GNOME/libadwaita look

Research notes for the redesign, 2026-07-04. Two tracks: (A) what FanControl
(Rem0o, Windows) actually does so the daemon can match its behavior model, and
(B) the concrete GNOME HIG / libadwaita rules and values the React GUI must
follow to pass as a native GNOME app.

---

## A. FanControl behavior model

### A.1 Object model — three first-class lists

FanControl's home screen is exactly three collections, all user-nameable:

1. **Controls** — one per controllable fan header. Each picks *one* curve and
   holds hardware-facing params (step limits, start/stop %, offset, minimum).
2. **Curves** — named behaviors. A curve references a temperature *source*
   (or other curves, for Mix/Sync). Multiple controls can share a curve.
3. **Sensors** — temp sources, including *custom* derived sensors.

This separation is the big architectural idea: curves are reusable values,
controls are bindings of curve → hardware. fand's config is close (named
`[curves.*]` + `[channels.*]`) but fand fuses "policy" into the channel;
FanControl instead makes Mix *a curve type*, so a channel is always just
"channel → one curve".

### A.2 Curve types

| Type | Computes | Parameters |
|---|---|---|
| **Graph** | Point-based map, linear interp | temp source; points; hysteresis; response time; "ignore hysteresis at extremes" |
| **Linear** | Ramp between two points | temp source; min/max temp; min/max %; same hysteresis params |
| **Mix** | Combine *other curves'* outputs | function: **Max / Min / Average / Sum / Subtract**; list of member curves |
| **Trigger** | Latching two-state | idle temp + idle %; load temp + load %; response time. Holds current speed between thresholds |
| **Flat** | Constant % | the % |
| **Sync** | Mirror another control's output | source control; offset (absolute or proportional %) |
| **Auto** | Seeks equilibrium toward a target temp | idle temp (min-speed zone); load temp (target); deadband °C; step rate; response time (doubled when decreasing); min/max % |
| *(RPM mode)* | Any curve can output target RPM instead of % | requires calibration data (RPM↔% map per fan) |

Key detail on **Mix**: it mixes *curve outputs*, not temperatures — i.e. it is
exactly fand's max-of-outputs invariant, generalized to five functions. Each
member curve still evaluates at its own sensor.

### A.3 Hysteresis and response time (per curve, on the input)

From the curve editor (visible in the screenshot: "Hysteresis ⇅ 2 °C · 1 sec"):

- **Hysteresis (°C)** — the *input temperature* must move by at least this
  delta from the last accepted value before the curve recomputes. Newer
  versions split it into separate **up** and **down** deltas.
- **Response time (s)** — the change must *persist* this long before it's
  accepted (a hold-off timer, not a moving average).
- **Ignore hysteresis at extremes** — bypass both at the ends of the curve so
  the fan still reaches 100 % promptly and settles to idle fully.

Contrast with fand today: fand smooths the temp (rolling average) and applies
a deadband on the *output PWM*. FanControl gates on the *input temp* and does
no averaging by default (averaging is opt-in via a custom sensor, below).
Input-side hysteresis is what makes its fans feel "quiet" — the curve output
is frozen until the temp genuinely moves.

### A.4 Control-level parameters (hardware side, per fan header)

- **Step up % / Step down %** — max output change per update tick, asymmetric
  (fand has this ✓).
- **Start %** — kick value used to spin a stopped fan back up (fand
  deliberately has no equivalent — zero-RPM mode was removed 2026-07-06,
  fans never stop).
- **Stop %** — below this computed value, snap output to 0 (this is how
  zero-RPM is expressed; not adopted — see Start %).
- **Offset %** — added to the curve's output for this control only (lets two
  fans share one curve with a bias).
- **Minimum %** — hard floor (fand's `min_pwm` ✓).

### A.5 Custom sensors (derived inputs)

- **Time average** — rolling average of a sensor over N seconds (this is
  where fand's per-channel `smoothing_seconds` lives in FanControl's model —
  as a *sensor*, so any curve can use the smoothed or raw variant).
- **Mix sensor** — Max/Min/Avg/Sum/Subtract over several *temperatures*
  (distinct from Mix curve; useful for e.g. "hottest NVMe").
- **Offset sensor** — sensor ± constant or ± proportional %.
- **File sensor** — reads °C from a file (their escape hatch for external
  sources; fand's NVML input is our equivalent, but a file/command sensor is
  a cheap extensibility win on Linux).

### A.6 Calibration & assisted setup

Per-fan calibration sweeps the PWM range and records: the **stop point**, the
**start point** (min % that reliably spins from 0), and a full **RPM↔% graph**
(enables RPM mode). The UI marks calibrated controls with a badge. Assisted
setup at first run detects which control moves which fan.

### A.7 Gap analysis — what this implies for fand

Already matching: named curves + channels, max-of-outputs mix, asymmetric
ramp, min floor, per-channel smoothing. (Kick-on-start matched at the time;
it left with zero-RPM mode, 2026-07-06.)

Worth adopting (roughly in order of value):

1. **Per-curve hysteresis on input temp + response time** (up/down deltas,
   ignore-at-extremes). Replaces/augments the output deadband; pure
   `fand-core` logic, very testable.
2. **Mix as a curve type** with Max/Min/Avg (Sum/Subtract are foot-guns;
   Max stays the safety-documented default). Channel then always binds one
   curve — simplifies the channel model.
3. **Flat curve type** — trivial, useful for testing and for the GUI.
4. **Trigger curve type** — nice for a semi-passive case fan; if added,
   forbid on pwm1 like the pump floor, and idle_pwm keeps the min_pwm floor
   (zero-RPM mode was removed 2026-07-06 — fans never stop).
5. **Per-channel offset %** — cheap, lets pwm1/pwm2 share a curve later.
6. **Calibration (`fanctl calibrate pwm2`)** — sweep down to find stall/start
   points to *inform* min_pwm config. Constraints: interactive only,
   refuse on pwm1 (pump inline), clamp test floor, timeout back to curve.
7. **Time-average as explicit smoothing config on the sensor** rather than
   the channel — matches FanControl's model; optional refactor.

Probably skip: **Auto curve** (feedback controller writing to real hardware =
oscillation risk, contradicts "predictable curve" philosophy), **RPM mode**
(needs calibration data + closed loop; % mode is fine), **Sync curve** (only
two channels on this board), **Sum/Subtract** mixes.

### A.8 UI concepts worth stealing (GNOME-ified in part B)

- Home = two card rows: **Controls** (name, curve dropdown, live % + RPM,
  enable toggle) and **Curves** (mini sparkline preview of the curve shape,
  current source temp, current output %).
- Curve editor opens as a focused dialog: big graph, draggable points,
  numeric fields for the selected point, source + hysteresis at top.
- Everything renameable in place; calibrated/enabled state visible as badges.

---

## B. GNOME HIG / libadwaita — making the React app native-looking

### B.1 Structural patterns (HIG)

- **No menu bar, no status bar.** One **header bar** with window controls; on
  content-heavy pages it may be *flat* (same bg as content, no border — the
  ToolbarView "flat header bar" style). Primary menu (⋮) on the right holds
  About/Settings if not in a sidebar.
- **Sidebar navigation** (`AdwNavigationSplitView` + `.navigation-sidebar`):
  sidebar bg is darker than content, selected row uses a **neutral**
  (white-alpha) highlight, *not* accent color. Rows are rounded and padded.
- **Boxed lists** (`.boxed-list`) for all settings/forms: cards containing
  rows — `AdwActionRow` (title + subtitle + trailing widget), `SwitchRow`,
  `SpinRow`, `ComboRow`, `ExpanderRow` (a toggle row that expands to its
  detail rows). Lists have max width (~576px in prefs pages), centered.
- **Cards** (`.card`) for dashboard content; can be `.activatable` (hover
  state) if clickable.
- **Feedback:** toasts (bottom, floating pill) for "config applied";
  `AdwBanner` (full-width bar under header, warning color) for persistent
  states — override active, failsafe engaged, daemon unreachable.
  `AdwStatusPage` (big centered icon + title + description) for empty/error
  full-page states.
- **Instant apply.** GNOME settings apply immediately — no Ok/Cancel row of
  buttons at the bottom of a page. Exception: a *dialog* editing a compound
  object (our curve editor) may keep Cancel/Apply in its header bar, with
  the affirmative button `.suggested-action`.
- **Dialogs** are floating sheets (`AdwDialog`), rounded 15px, dimmed
  backdrop, sized to content — not fullscreen takeovers.
- Window controls: respect the system layout; on niri/Wayland Tauri draws
  CSD — keep close button only if that matches the user's gnome setting.

### B.2 Dark palette (libadwaita CSS variables, dark values)

| Variable | Dark value |
|---|---|
| `--window-bg-color` | `#222226` |
| `--window-fg-color` | `#ffffff` |
| `--view-bg-color` | `#1d1d20` |
| `--headerbar-bg-color` | `#2e2e32` |
| `--sidebar-bg-color` | `#2e2e32` |
| `--card-bg-color` | `rgb(255 255 255 / 8%)` |
| `--popover-bg-color` | `#36363a` |
| `--dialog-bg-color` | `#36363a` |
| `--shade-color` | `rgb(0 0 6 / 25%)` |
| `--headerbar-shade-color` | `rgb(0 0 6 / 36%)` |
| `--accent-bg-color` (blue, default) | `#3584e4` |
| `--accent-color` (standalone, dark) | `#81d0ff` |
| `--destructive-bg-color` | `#c01c28` |
| `--success-bg-color` | `#26a269` |
| `--warning-bg-color` | `#cd9309` |

Light-theme counterparts: window `#fafafb`, view `#ffffff`, headerbar
`#ffffff`, sidebar `#ebebed`, card `#ffffff`, fg `rgb(0 0 6 / 80%)`.

System accent hues (GNOME 47+, user-selectable): blue `#3584e4`, teal
`#2190a4`, green `#3a944a`, yellow `#c88800`, orange `#ed5b00`, red
`#e62d42`, pink `#d56199`, purple `#9141ac`, slate `#6f8396`. Standalone
(text/icon) variants are lightened programmatically in dark mode for
contrast — hardcoding blue's `#81d0ff` is fine for now.

Rule of thumb: **accent is for interaction states and the suggested action
only** — not decoration. Charts/status colors come from the GNOME palette;
dim secondary text with opacity (`.dimmed`, `--dim-opacity` ≈ 55%), never
with grey hex values.

### B.3 Typography

Fonts (GNOME 48+): UI = **Adwaita Sans** (customized Inter), mono =
**Adwaita Mono** (Iosevka). Web stack:
`"Adwaita Sans", "Inter", system-ui, sans-serif` and
`"Adwaita Mono", "Iosevka", monospace`. Base size ≈ 11pt (≈14.7px).

Exact classes from the libadwaita stylesheet (sizes relative to base):

| Class | Size | Weight | Notes |
|---|---|---|---|
| `.title-1` | 181% | 800 | page-level hero only |
| `.title-2` | 136% | 800 | |
| `.title-3` | 136% | 700 | |
| `.title-4` | 118% | 700 | card titles |
| `.heading` | 100% | 700 | row titles, group headers |
| `.body` | 100% | 400 | line-height 140% |
| `.caption-heading` | 82% | 700 | |
| `.caption` | 82% | 400 | subtitles, dim + caption |
| `.numeric` | — | — | `font-variant-numeric: tabular-nums` |

**Use `.numeric` (tabular figures) on every live readout** — temps, RPM, % —
so values don't jitter horizontally as digits change. Inter has proper
tabular figures; this is a one-line CSS fix with visible payoff.

### B.4 Metrics (from libadwaita stylesheet source)

- Corner radii: **buttons/menus 9px**, **cards & boxed lists 12px**,
  **popovers 15px**, **dialogs/windows 15px**.
- Buttons: min-height 24px, padding `5px 10px` (⇒ ~34px tall at 100% text);
  `.pill` horizontal padding 17px; `.circular` fixed 34×34.
- Boxed-list rows: min-height 50px, header content padding `8px 12px`,
  horizontal padding 12px.
- List/preferences group header: ~18px top / 6px bottom spacing.
- Spacing rhythm: multiples of **6px** — 6 tight, 12 standard, 18 group,
  24 page margins. Page content margins ≥ 12px, prefs pages center content
  at max ~580px width.
- Transitions: ~200ms ease-out is the libadwaita norm.

### B.5 Component/style-class cheat sheet to emulate in CSS

- `.card` — card-bg, 12px radius, subtle shadow (`--shade-color`).
- `.boxed-list` — card look on a list; rows separated by hairline
  (`--border-color` ≈ fg at 15% alpha); first/last rows round the corners.
- `.navigation-sidebar` rows — 6px radius (inside), neutral selection
  (`fg 10% alpha`), hover `fg 7% alpha`.
- `.flat` button — transparent until hover (`fg 7%`), active `fg 16%`.
- `.suggested-action` — accent-bg + white fg; `.destructive-action` same
  with destructive-bg. Both also combine with `.flat`/`.pill`/`.circular`.
- `.osd` — dark translucent overlay controls (curve-editor toolbar).
- Switches: pill track, 26px tall; checked = accent-bg.
- Sliders (`GtkScale`): 4px trough at `fg 15%`, accent fill, 20px round knob.

### B.6 What this means for the fand GUI concretely

1. Adopt the palette/typography/metrics above as CSS custom properties in
   `index.css` (mirror libadwaita variable *names* so future theming is a
   value swap). Support light theme eventually via `prefers-color-scheme`.
2. Dashboard: FanControl's layout, GNOME's skin — "Controls" and "Curves"
   sections (`.heading` section titles) with `.card` grids; live values in
   `.numeric`; curve cards show a mini preview sparkline.
3. Curve editor: `AdwDialog`-style floating sheet (15px radius, backdrop
   dim, Cancel / Apply-suggested in its header) — or inline page; graph
   colors from chart palette, points in accent.
4. Settings: real boxed lists — switch rows, spin rows (no zero-RPM
   controls: the mode was removed 2026-07-06).
5. Status/feedback: toast on config apply; warning `AdwBanner` when an
   override is active or the daemon socket drops; `AdwStatusPage` when
   disconnected.
6. Sidebar stays, restyled to `.navigation-sidebar` spec (neutral selection,
   `#2e2e32` bg, window bg `#222226`, content on `#1d1d20` where "view-like").

### Sources

- FanControl official docs: <https://getfancontrol.com/docs/>
- FanControl repo/discussions: <https://github.com/Rem0o/FanControl.Releases>
- GNOME HIG: <https://developer.gnome.org/hig/>
- Libadwaita CSS variables: <https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/css-variables.html>
- Libadwaita style classes: <https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/style-classes.html>
- Libadwaita stylesheet source (typography/metrics/radii):
  <https://gitlab.gnome.org/GNOME/libadwaita/-/tree/main/src/stylesheet>
- Adwaita fonts (GNOME 48): <https://blogs.gnome.org/monster/introducing-adwaita-fonts/>
