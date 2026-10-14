# Devlog: Stream B (Player & widgets)

Worktree `.claude/worktrees/epic-leavitt-ce2e61`, branch `claude/epic-leavitt-ce2e61`, started from `protocol-v0.1` (6d02d71). Target dir `target/agent-b`.

## 2026-10-14 · Phase 7a plan: UI shell

- `crates/jidr-ui/build.rs` reads `docs/design/tokens.json` and writes `ui/theme.slint` (`export global Theme`), dark and light values side by side, picked by `Theme.dark`. The file is checked in and rewritten by every build when the tokens change, so it cannot drift. Names follow D41 (`jidr-color-surface-raised` → `Theme.surface-raised`).
- Fonts: Geist, Geist Mono (vercel/geist-font v1.7.2) and Noto Kufi Arabic (google/fonts) TTFs with their OFL texts in `apps/jidr-gui/assets/fonts/`, imported by `ui/app.slint`.
- Icons: the Lucide subset we use in `crates/jidr-ui/ui/icons/` (lucide-static 1.57.0, ISC, stroke rewritten to 1.75), drawn with `colorize`.
- Components from `docs/design/system/components/*`: Button, IconButton, StatusPill, RootBadge, Kbd, Tab/TabStrip, SidebarItem/DeviceRow, SearchField, Toast. Strings via `@tr`.
- RTL: Slint 1.18 has no layout direction and the software renderer can't mirror with transforms, so `Theme.rtl` drives explicit mirroring: regions swap sides (sidebar, toolbar, panels, traffic lights), rows place leading/trailing items by `x`, text aligns to the start edge.
- Daemon connection: a `Bridge` owns a tokio runtime, `Client::connect_or_spawn`, subscribes to `device.` and `job.` events and retries every 2 s after a disconnect; the UI shows Connecting / Reconnecting.
- Snapshot harness: `cargo run -p jidr-ui --example snapshot -- [--check|--update]` renders every screen state with Slint's software renderer on a `MinimalSoftwareWindow` (no display needed), in dark, light and RTL, and compares against `crates/jidr-ui/tests/golden/*.png` (per-channel tolerance, small differing-pixel budget).

## 2026-10-14 · Phase 7a done: UI shell

Built:
- `build.rs` generates `ui/theme.slint` (`Theme`: 190 colour/size/duration tokens, dark+light side by side, Reduce Motion values from `bundle.css`; `Easing` in its own global because easing can't cross into Rust) and `ui/icons.slint` (one `@image-url` per Lucide SVG). Both are rewritten only when they change.
- Fonts (Geist, Geist Mono, Noto Kufi Arabic variable) + OFL texts in `apps/jidr-gui/assets/fonts/`; 101 Lucide icons (lucide-static 1.57.0, stroke 1.75) + ISC licence in `crates/jidr-ui/ui/icons/`.
- Components (`ui/components/base.slint`, `status.slint`): text roles, Icon, Spinner, Pill, Tag, Kbd, Button (4 kinds, sm/md, icon, kbd), IconButton, Bar, Panel, Toggle, Checkbox, Segmented, TextField, Scroll, TrafficLights, Toast, SectionHeader, KV, StatusPill/StatusText, RootBadge (all seven shapes + Beta tag).
- Shell (`ui/shell.slint`, `launcher.slint`, `app.slint`): title bar with Library + device tabs and connection pill, sidebar (command field, devices, jobs, SDK state, settings), mini sidebar for device tabs, device detail, first-run empty state.
- `model.rs` builds every label (PRODUCT §5 statuses, number rules) from daemon data with an injected clock; `fixtures.rs` runs the §7 sample data through it.
- `bridge.rs`: `Client::connect_or_spawn`, `events.subscribe(device., job.)`, reload on events (80 ms coalescing), `daemon.status` health check every 3 s, retry every 2 s. `tests/bridge.rs` checks load → event-driven reload → daemon gone → Reconnecting → new daemon → Online against `TestDaemon`.
- Snapshot harness: `offscreen.rs` (software renderer on `MinimalSoftwareWindow`) + `examples/snapshot.rs` (`--check` / `--update` / `--only`; channel tolerance 24, 0.2 % pixel budget).
- RTL: Slint 1.18 has no layout direction and its software renderer ignores transforms, so `Theme.rtl` drives explicit `x` placement; regions, rows, badge order, bars and toggles mirror; code/serials stay LTR.

Validation: `cargo nextest run -p jidr-ui` (8 passed) and `cargo run -p jidr-ui --example snapshot -- --check` (12 renders ok). `cargo xtask ci` ok.

Open: `jidr daemon` isn't implemented yet (A's 5a), so the live GUI can't start a daemon on its own (gap B1). On macOS the window uses the native traffic lights over a transparent title bar; dragging calls winit `drag_window`.
