# fand GUI

Tauri 2 + React + TypeScript desktop dashboard for the fand daemon.
Runs unprivileged as the user (socket is group-`fand`); the Rust backend
(`src-tauri/`) subscribes to the daemon's status stream via
`fand_proto::client` and re-emits frames as Tauri events for React.

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

- `daemon/` — everything that talks to the Rust backend: the TypeScript
  mirror of the wire types and the Tauri-event hook. The GUI's twin of
  `fand-proto`; write commands land here too when they arrive.
- `dashboard/` — the read-only live view (temp chart, channel cards).
- App shell (`main.tsx`, `App.tsx`, `index.css`) stays at the root.

Future slices add sibling folders (`curves/` for the editor, `settings/`
for the channel panel); a `components/` folder appears only once two
features actually share a piece.

Roadmap (PLAN.md §7): dashboard (this slice) → curve editor → channel
settings panel.
