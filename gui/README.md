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

`src/` is organized by feature, one folder per concern:

- `daemon/` — the TypeScript mirror of the backend's payload types and
  the one event hook (`useDaemonStatus`) every piece of daemon state
  flows through.
- `dashboard/` — the live view (temp chart, channel and curve cards).
- `curves/`, `settings/` — thin write-command wrappers (`writes.ts`);
  fire-and-report, no state.
- `dialogs/` — the curve editors, channel properties, new-curve,
  preferences and about dialogs.
- `adw/` — the hand-rolled libadwaita-style component library.
- `shell/` — CSD headerbar, accent handling, persisted prefs.
- App shell (`main.tsx`, `App.tsx`, `index.css`) stays at the root.
