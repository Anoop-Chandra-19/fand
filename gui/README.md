# fand GUI (phase 6 — not scaffolded yet)

Tauri 2 + React + TypeScript. Scaffold when phases 1–5 are done:

```fish
cd gui
npm create tauri-app@latest . -- --template react-ts
```

Design notes from the plan:
- Tauri Rust backend reuses `fand-proto` client code; frontend talks to it via
  Tauri commands/events. Zero privileges needed (socket is group-`fand`).
- Curve editor: SVG draggable points, live interpolation preview, `set_config`
  on drag-end.
- Live dashboard: rolling charts of temps + per-fan RPM/PWM fed by
  `subscribe_status` events.
- Channel panel: policy, sensor+curve assignment, min_pwm, smoothing,
  zero-RPM toggle with kick settings.
- Native Wayland; verify it behaves on niri.
- Visual states for "override active" and "sensor failure / failsafe engaged".
