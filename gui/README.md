# fand GUI

Tauri 2 + React + TypeScript desktop dashboard for the fand daemon.
Runs unprivileged as the user (socket is group-`fand`).

The Rust backend (`src-tauri/`) is the service layer: it subscribes to
the daemon's status stream via `fand_proto::client` and owns the only
config copy in the process (`src-tauri/src/state.rs`). Every `status`
event carries the frame plus the newest same-instance config covering
it (right after writes, config may transiently run ahead of queued
frames — never behind, never across a daemon restart); writes emit a
`config` event with the applied result and return any
applied-with-caveat warning in their invoke result (one toast per
operation, no cross-channel ordering), `daemon-down` repeats while the
socket is dead, and `daemon-restarted` closes draft dialogs when a
restart is detected mid-stream. Config versions are the daemon's `(instance,
generation)` pair — never compared across instances (see `state.rs`).
Writes are serialized by one gate and sent as compare-and-set, so
concurrent edits (including fanctl's) conflict instead of silently
overwriting each other. React is a pure presentation layer — it renders
the last event and never fetches, caches, or reconciles daemon state.

```fish
cd gui
npm install
npm run tauri dev     # against the live daemon socket
npm run tauri build   # release binary
```

Point it at a dev daemon instead with
`FAND_SOCKET=/tmp/fand-dev.sock npm run tauri dev`.

System prerequisite on Arch/CachyOS: `webkit2gtk-4.1`.

`src-tauri` is intentionally excluded from the root cargo workspace so
daemon-side `cargo test/clippy --workspace` stay fast.

## Frontend layout

`src/` is organized by ownership rather than by UI shape. A feature owns
its cards, dialogs, commands, and pure model logic together:

- `app/` — composition and lifecycle: the window shell, dashboard layout,
  app-level dialogs, and local preferences. `App.tsx` owns daemon lifecycle,
  transient UI state, and dialog orchestration; `Dashboard.tsx` renders the
  main view.
- `features/curves/` — curve cards, previews, editors, write commands, and
  client-side curve evaluation.
- `features/fans/` — channel cards, channel properties, and channel commands.
- `features/monitoring/` — live telemetry visualizations.
- `features/pwm.ts` — pure PWM display conversion shared by fan and curve UI.
- `api/` — Tauri/daemon contracts, event hooks, command adapters, and shared
  write-result normalization. It contains no visual components.
- `adw/` — reusable hand-rolled libadwaita-style primitives. It contains no
  fand domain behavior.

Dependencies flow from `app` to `features`, `api`, and `adw`; features may
use `api` and `adw`. Cross-feature imports should be one-way and reflect a
real domain dependency (for example, a fan channel may render a curve
preview). Generic domain helpers belong at the `features/` root rather than
inside one feature.

A substantial new capability gets a sibling feature folder only when work
on it starts. For example, GPU clock tuning would live in
`features/gpu-tuning/` with its view, cards/dialogs, commands, and model
colocated; any new Tauri contracts belong in `api/`.
