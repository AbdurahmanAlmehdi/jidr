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

## 2026-10-14 · Phase 7b plan: launcher, first run, create/edit

- Setup window (`ui/wizard.slint`, 760×580, Setup.dc.html): 1 find SDK (`sdk.status`: Studio SDK, `$ANDROID_HOME`, `$ANDROID_SDK_ROOT`), 2 Google's license from `sdk.licenses` shown unchanged, "I accept" enabled only after scrolling to the end, then `sdk.accept_license {id, sha256}`; 3 `image.install` job with real bytes/progress; 4 first device (reuses the create form).
- Create/edit sheet (`ui/device_form.slint`, Create.dc.html): name, profile (`profile.list`), Android version list (`image.list`, host ABI only, installed/downloading/download states), root-mode cards (default Shell root where the image allows it, Full root highlighted, Play Store: Shell disabled + Beta + Play Integrity note; D7), RAM/CPU/screen, "Start after creating". Edit = same sheet over `device.edit`.
- Settings window (`ui/settings.slint`, Settings.dc.html): General (default root mode, map tiles, appearance, quick boot), Storage (SDK path and state, per-device disk from `device.details`, images, cache), Agents (copyable MCP snippets), About (licences incl. Lucide and the fonts). Settings with no protocol command yet (default root mode, quick boot) are kept in a GUI prefs file and filed as a gap.
- Command palette (`ui/palette.slint`, ⌘K): filtered list of actions and devices; ↑/↓/↵/Esc.
- Shortcuts: ⌘K, ⌘N, ⌘,, ⌘1…9, ↵ open, Esc closes overlays.
- Goldens: launcher empty / populated / downloading / error, every setup step, create (Google APIs, Play Store), edit, each settings tab, palette.

## 2026-10-14 · Phase 7b done: launcher, first run, create/edit

Built:
- Setup window (`ui/wizard.slint`, `src/wizard/`): detection from `sdk.status` + env, download plan from `image.list` sizes (picks API 35 Google APIs, else newest supported), license text from `sdk.licenses` shown unchanged with "I accept" enabled only after scrolling to the end, consent per hash via `sdk.accept_license`, then `image.install` (or `sdk.install`) as a job with real progress, ETA and failure text; step 4 creates the first device.
- Create/edit sheet (`ui/device_form.slint`, `src/device_form/`): profiles, host-ABI image list (installed / download / downloading), root cards with D7 defaults (Shell where allowed, Full highlighted, Play Store disables Shell, shows Beta and the Play Integrity note), RAM/CPU/screen with validation, only overrides sent to `device.create`; a missing image is installed first. Edit sends changed fields to `device.edit`.
- Settings window (`ui/settings.slint`, `src/settings/`): General (default root, map tiles incl. custom URL, appearance, quick boot; in `gui-prefs.json`, gap B4), Storage (SDK path/state, per-device disk, images with device counts, cache with Clear), Agents (Add to Claude Desktop merges `claude_desktop_config.json` without touching other servers; copyable snippets), About (licences incl. Lucide + fonts).
- Command palette (`ui/palette.slint`, `src/palette/`): open/start/stop/edit per device, new device, setup, settings, appearance; ordered-letters fuzzy filter; ↑/↓/↵/Esc. "More actions" opens it narrowed to the device.
- Shortcuts: ⌘K, ⌘N, ⌘,, ⌘W, ⌘0, ⌘1…9, ↵ opens the selected device, Esc closes overlays. Confirmation dialog for destructive or slow actions (revert root).
- Slint gotcha recorded: inside `@children`, `parent` is the component that received them, not the slot container; sheets and settings rows expose their content size instead.

Validation: `cargo run -p jidr-ui --example snapshot -- --check` → 57 renders ok; `cargo nextest run -p jidr-ui` → 21 passed; `cargo xtask ci` ok.

Not done: the manual first-run walkthrough needs a daemon the GUI can spawn (`jidr daemon`, gap B1); `--example dev-daemon` (7c) covers live runs meanwhile. Gaps B4 (daemon settings) and B5 (job bytes) filed.

## 2026-10-14 · Phase 7c plan: player window

- Device tab per Device.dc.html (4A): mini sidebar, stage (bezel from `frame-*` tokens, fit or pixel-perfect 1:1 in a scrolling stage), 48 px toolbar, docked widget panel host, logcat bar/panel.
- Transport per ADR-0001: `device.connect_info` → `EmulatorController` directly; `streamScreenshot` RGBA8888 with both sides set at the screen's physical size (capped at the device size), one UI hop in flight with a newest-frame slot, re-subscribe on resize/rotation; frames are top-down.
- Input: pointer → `streamInputEvent` touches in device px; ⌥ drag pinch (mirrored second finger), ⇧⌥ drag two-finger pan; wheel → `injectWheel`; keys → `sendKey` (W3C key names, text as keypress). Toolbar: back/home/recents/volume/power as keys, rotate via the physical model, screenshot/record via `media.*` (10a), pixel-perfect, shared clipboard (`streamClipboard` ↔ pasteboard).
- Drag-drop through winit file events: apk → `file.install` (replace) then `app.launch`; signature mismatch → confirm, `app.uninstall`, reinstall; zip on Full root → `root.module.install` (falls back to push, gap B6); other → `file.push`.
- New widgets commands needed now: `app.inspect` (package from the APK's binary manifest, no SDK tools), `app.launch`, `app.uninstall`.
- Measurement: the debug overlay (⌥⌘I) shows fps, frame age and touch→frame; `JIDR_PLAYER_BENCH` drives 30 s of Settings scrolling through the player's own input channel and writes the numbers; `JIDR_PLAYER_DROP` hands a file to the drop handler.

## 2026-10-14 · Phase 7c done: player window

Built:
- `ui/player.slint`: device tab (mini sidebar, stage with token bezel, fit and pixel-perfect 1:1 with a scrolling stage and chip, 48 px toolbar, widget panel host, logcat bar and panel with level/text filters and follow, APK drop ring + label, debug overlay). Device-tab root badge and serial in the title bar. A drawn sample app (`MockScreen`) stands in for frames in goldens, so no Android UI is in the repo.
- `src/player/`: `session.rs` (direct `EmulatorController` connection from `device.connect_info`; RGBA8888 stream at the screen's physical size capped at the device size, both sides set, newest-frame slot with one UI hop in flight, re-subscribe on resize/rotation, input stream, clipboard sharing both ways, stats), `input.rs` (device-px mapping, ⌥ pinch / ⇧⌥ two-finger pan, W3C keys, wheel), `drop.rs` (apk / module zip / push routing and copy), `logcat.rs` (threadtime parser, filters), `bench.rs` (dev measurement), `mod.rs` (session per active tab, toolbar, shortcuts ⌥⌘I overlay, ⌥⌘0 pixel-perfect, ⌘L logcat, ⌘J panel, ⇧⌘S screenshot; winit file-drop events).
- `jidr-widgets` starts: `provider()` (gap A1) with `app.inspect` (package from the binary manifest, no SDK tools), `app.launch`, `app.uninstall`; `examples/dev-daemon.rs` (real daemon + widgets for live runs, gap B1); `tests/fixtures/build-hello-apk.sh` builds a tiny test app with the SDK's build tools (read-only).

Manual check (the plan's 7c validation), 2026-10-14, MacBook Pro M1 Pro, external 3440×1440 display (scale 1.0), emulator 35.6.11, API 31 `google_apis` arm64 AVD "Pixel-ish 6.1" (1080×2400, created through the daemon from Studio's SDK read-only), `-gpu host`, release `jidr-gui`:
1. Started "Pixel-ish 6.1" from the GUI (`JIDR_PLAYER_DEVICE` opens its tab); booted in 24.9 s.
2. Scrolled Settings for 30 s through the player's own input channel (`JIDR_PLAYER_BENCH`; 59 alternating 300 ms drags): **active fps median 60.0, p10 56.7**; received 59.8 fps, shown 59.8 fps, 61 MB/s at 339×753 frames (the screen's physical size); frame age median 1.6 ms (p90 1.8); **touch → next frame median 8.3 ms, p90 14.5 ms**; player CPU 15%. Debug build: 59.9 fps, 8.4 ms, 94% CPU. Budget (≥ 30 fps, ≤ 80 ms) met.
3. Dropped `hello.apk` (`JIDR_PLAYER_DROP`, the same handler winit's `DroppedFile` calls): `file.install` → `app.launch`; `adb shell dumpsys activity activities` → `mResumedActivity … ly.jidr.hello/.MainActivity`.
Not covered by a human yet: a physical Finder drag and a real trackpad scroll (the same handlers, driven from code here). A window-only capture (`screencapture -l`) showed the live Settings screen in the device tab with the toolbar, panel and logcat bar.

Validation: `cargo nextest run -p jidr-ui -p jidr-widgets` (36 passed), `cargo run -p jidr-ui --example snapshot -- --check` (81 renders ok incl. 8 player states × 3), `cargo xtask ci` ok.

Open: rotation's touch mapping and the multi-touch gestures were exercised only by unit tests (to check with 8a's `sensor.rotate` on the emulator). Gap B6 (module zip install comes with C's 9d).

## 2026-10-14 · Phase 8a plan: core widgets

- `jidr-widgets` commands (all through the daemon, so CLI and MCP get them too): `battery.get/set` (gRPC), `sensor.rotate/set/get` (physical model; motion sensors via `setSensor`), `network.get/set` (`svc wifi|data`, console speed/delay profiles), `phone.call/sms` (gRPC) and `phone.gsm` (signal bars via `gsm signal-profile`, R3; data/voice state), `key.press` (W3C keys), `dev.get/set` (show taps, pointer location, layout bounds, animation scales), `clipboard.get/set`, `fingerprint.touch/remove` (console `finger`, R8), `camera.get/set` (`-camera-back/-front` in the device's launch args, R7, next start).
- Panels in `crates/jidr-ui/ui/widgets/` laid out like Device.dc.html: Battery, Network, Sensors (rotation + motion/environment), and a ⋯ menu for Phone, Controls, Dev tools, Clipboard, Biometrics, Camera.
- `tests/widgets_emulator.rs` (`--features emulator`): a real daemon with the widgets provider boots one device from `JIDR_TEST_IMAGE` and checks every command with an adb-observable assertion (`dumpsys battery`, `settings get`, `getprop`, `dumpsys telephony.registry`, `adb emu …`). Emulator tests serialise on a file lock so parallel nextest runs don't boot several emulators at once.

## 2026-10-14 · Phase 8a done: core widgets

Built (`jidr-widgets`, all daemon commands): `battery.get/set`, `sensor.rotate/set/get`, `network.get/set` (7 named speed/delay profiles), `phone.call/sms/gsm` (signal bars 0–4 via `gsm signal-profile`), `key.press` (hardware buttons as evdev down/up, other keys as W3C names), `dev.get/set`, `clipboard.get/set`, `fingerprint.touch/remove`, `camera.get/set`. GUI panels (`ui/widgets/panels.slint`, `src/widgets/`): Battery (metric, slider + field, charging, quick set), Network (Wi-Fi, mobile data, profile list, signal bars), Sensors (rotation, motion presets, light/temperature/pressure/humidity, proximity), ⋯ (phone + SMS, D-pad + media keys, dev toggles + animation scale, clipboard, fingerprint, back camera). The toolbar's hardware keys and rotation now go through `key.press` / `sensor.rotate` like the CLI.

Findings on the API 31 arm64 emulator (35.6.11):
- Home as evdev `KEY_HOMEPAGE` (172) does nothing; the W3C name `GoHome` works. Back (158), Recents (580) and the volume keys work as evdev.
- Volume keys raise Android's volume panel but the emulator's stream volumes stay fixed, so the test checks the panel window.
- Rotation shows in `dumpsys window` (`mRotation=1`), not in `dumpsys input`'s `SurfaceOrientation`; the Pixel launcher is portrait-only, so the test rotates over Settings.
- `gsm signal-profile N` shows up as WCDMA `ss` (−105 dBm at 0 … −55 at 4).

Validation: `cargo nextest run -p jidr-widgets --features emulator` → 11 passed (`widgets_core_commands` 76 s incl. a 30 s cold boot). `cargo run -p jidr-ui --example snapshot -- --check` → 90 renders. `cargo xtask ci` ok (152 tests).

Open: fingerprint "recognised" needs a finger enrolled once in the guest (the panel says so); rich biometric scenarios stay post-MVP (D36).

## 2026-10-14 · Phase 8b plan + done: GPS

Plan: `gps.get/set` (gRPC `setGps`), `gps.route` (a job walking a GPX/KML track at 10 m/s × 1/2/4×, one fix per ≤ 50 m, progress text like the design), `gps.route_control` (speed, pause, loop) and `gps.route_stop`; GPS panel with an OSM tile map (configurable URL from Settings, attribution always visible, disk cache), coordinate fields, map click to move the fix, and the route card.

Built:
- `crates/jidr-widgets/src/gps/` (`track.rs`: tolerant GPX `trkpt/rtept/wpt` + KML `coordinates` / `gx:coord` parsing, haversine distance, bearing, walking a polyline; `mod.rs`: commands, route registry so a new route stops the old one).
- `crates/jidr-ui/ui/widgets/panels.slint` `GpsPanel` and `src/widgets/gps.rs` (Web Mercator, tile fetch with reqwest + `Jidr/<ver>` User-Agent, cache under `~/Library/Caches/Jidr/tiles`, compositing to a 584×300 map image, route drawn as a `Path`, macOS file chooser for routes).
- Design deviation: the design's "Accuracy" field became "Bearing". The AOSP emulator's `GpsState` has no accuracy (Android reports hAcc 5 m for every fix), and PRODUCT §6 lists bearing.
- Test app: `build-hello-apk.sh … locprobe` builds `ly.jidr.locprobe`, which requests GPS updates and logs each fix. Without a client asking for location, `dumpsys location` never shows a fix.

Validation: `cargo nextest run -p jidr-widgets --features emulator -- gps` → `gps_fix_and_route_replay` passed (67.6 s incl. boot): `Location[gps 32.895400,13.180198 …]` in `dumpsys location`; a 3-point GPX at 600 m/s produced probe fixes near point 1, then 2, then 3 with latitude only falling. Goldens 93, `cargo xtask ci` ok (160 tests). One OSM tile fetched with the app's User-Agent: HTTP 200 PNG.

## 2026-10-14 · Phase 10a plan + done: screenshots and recording

Plan: `media.screenshot` (gRPC `getScreenshot` as PNG, saved to `~/Pictures/Jidr` by default; the GUI also copies it to the pasteboard) and `media.record` (a job: the daemon streams RGBA frames from the emulator at the device size with the long side capped at 1280, encodes MP4/H.264 at a constant 30 fps, repeating the last frame while the screen is still, until `media.record_stop`, `max_seconds` or cancel, which discards the file). Encoder: AVFoundation's `AVAssetWriter` (VideoToolbox in hardware, D13) through the MIT/Apache `objc2` bindings, in one module (`media/mp4.rs`) that allows `unsafe` with a comment; macOS only, elsewhere the command explains why it can't record.

Built: `crates/jidr-widgets/src/media/{mod,mp4}.rs`; GUI `src/recording.rs` (toolbar screenshot → pasteboard via `osascript`, record toggle with a "Recording m:ss" pill from the job's progress; stop goes through `media.record_stop` and the job reports the file).

Validation: `cargo nextest run -p jidr-widgets --features emulator -- media` → `media_screenshot_and_five_minute_recording` passed (397.5 s): screenshot 720×1280 PNG; recording returned 9000 frames in 300.1 s; `ffprobe` → `codec_name h264`, `duration 300.000000`, `nb_read_frames 9000` (needs ≥ 8100). Unit test `writes_an_mp4` encodes 2 s directly. `cargo xtask ci` ok (164 tests); 96 goldens incl. the recording pill.

Not done: audio (post-MVP, D13). Rotating the device mid-recording pads/crops to the first frame's size.

## 2026-10-14 · Phase 10b plan + done: snapshot manager

Plan: `snapshot.list/save/load/delete` on the emulator's `SnapshotService`, with a root-mode tag per snapshot so a snapshot taken under another root mode is refused (D29); the Snapshots page per Snapshots.dc.html (banner, cards with the emulator's own `screenshot.png` thumbnails, save dialog with the tag preview, refusal dialog with "Set up … root" → `root.set`).

Built:
- `crates/jidr-widgets/src/snapshots.rs`: tags in `<avd>/jidr/snapshots.json` (atomic writes), list from `<avd>/snapshots/*` (works while stopped: size, time, thumbnail, `restorable`), save/load as locking jobs, delete over gRPC or from disk when stopped, name checks (no separators; `default_boot` is the quick-boot snapshot). Untagged snapshots (made outside Jidr) restore as usual.
- `crates/jidr-ui/ui/snapshots.slint`, `src/snapshots/` (page, dialogs, banner copy, D29 refusal). The golden-case module is now `src/goldens.rs` so `src/snapshots/` matches the plan's path.
- Gap B7: tags file and refusal shape, for Stream C's 9c/9d.

Validation: `cargo nextest run -p jidr-widgets --features emulator -- snapshot` → `snapshot_round_trip_and_cross_root_refusal` passed (55.6 s): marker `before` → save "Clean install" (tag `none`) → marker `after` → load → marker `before`; after the device's root mode changes to Shell root (what `root.set` does), the load is refused ("taken with Not rooted … Shell root now", `refused = root_mode`) and the list shows `restorable: false`; delete removes it. Goldens 105 (manager, refusal, save dialog × 3). `cargo xtask ci` ok (169 tests).

## 2026-10-14 · Stream B summary

Goal check (exit 0): `cargo nextest run -p jidr-ui -p jidr-widgets` (62 passed) `&& cargo run -p jidr-ui --example snapshot -- --check` (105 renders ok) `&& cargo nextest run -p jidr-widgets --features emulator` (26 passed in 534 s on the API 31 arm64 image, incl. the 5-minute recording). Phases 7a, 7b, 7c, 8a, 8b, 10a and 10b are Done in the plan, one commit each.

Done:
- GUI (`crates/jidr-ui`, `apps/jidr-gui`): generated theme and icons, bundled fonts, Workbench shell with tabs, launcher, first-run setup, create/edit sheet, settings, command palette, device window with the direct gRPC player (60 fps, touch → next frame 8.3 ms median in release), toolbar, drag-drop, logcat, widget panels, GPS map, recording clock, snapshot manager. 35 screen states × dark/light/RTL in goldens.
- Widgets (`crates/jidr-widgets`, Apache-2.0, no Slint): `provider()` (gap A1) with `app.*`, `battery.*`, `sensor.*`, `network.*`, `phone.*`, `key.press`, `dev.*`, `clipboard.*`, `fingerprint.*`, `camera.*`, `gps.*`, `media.*`, `snapshot.*`; each checked on a real emulator. `examples/dev-daemon.rs` for live runs.

Deferred / not done:
- Human-only checks: a physical Finder drag and a trackpad scroll in the player (same handlers driven from code), and a first-run walkthrough with a real 2 GB download (the GUI can't start a daemon yet, gap B1).
- Recording audio (D13, post-MVP); the device rotating mid-recording keeps the first frame's size.
- Fullscreen window mode; live appearance switching follows macOS only at launch (System) or on the Settings toggle.
- Out of scope per the brief: identifiers, audio injection, rich biometric scenarios, every 🔎 row.
- RTL is explicit mirroring (Slint 1.18 has none); a few button groups (Setup footer) keep LTR order inside the mirrored layout.

Open gaps (docs/gaps.md): A1 (A registers `jidr_widgets::provider()`), B1 (`jidr daemon`), B2 (lockfile: slint-build, slint `unstable-winit-030`, reqwest in jidr-ui; zip + objc2 AVFoundation crates in jidr-widgets), B3 (root UI slots for C), B4 (daemon-side settings), B5 (job bytes), B6 (`root.module.install` for zip drops), B7 (snapshot tags for C).

## 2026-10-14: merge with Stream A, first CI run on PR #3

Merged `origin/stream/a-core` and `origin/main`. In gaps.md, A's G6/G7 replace my A1/A2 rows, and B1 is done (`jidr daemon` exists; widget commands appear once A registers the provider, G6). `Cargo.lock`: A's lock plus Stream B's 18 packages.

The first GitHub CI run found three Stream B problems my Mac runs couldn't show:
- Linux clippy: `lcd_size`, `Latest` and `Instant` are only used by the macOS recorder, so they were dead code on Linux. Now `cfg(target_os = "macos")`.
- macOS clippy on the runner: its SDK's newer `emulator_controller.proto` marks `hardwareConfig` deprecated (the vendored copy and emulator 35.6.11's don't). `#[allow(deprecated)]` on the one read, with a comment.
- Snapshots on the Linux emulator: `snapshot.save "Clean install"` was stored as `Clean_install`, so the tag keyed by the typed name went missing (`root_mode: null`, no D29 refusal). Jidr now picks the folder id itself (`snapshot_id`: ASCII letters, digits and `-._` stay, other ASCII → `_`, other characters → `x<hex>` so Arabic names stay distinct), keys tags by id with the typed name as `label`, adds a `-2` suffix when a different name maps to a taken id, and resolves names through labels in load/delete. Regression test: `ids_are_folder_safe_and_names_resolve_through_labels`. `snapshot_round_trip_and_cross_root_refusal` passes again on API 31 arm64 (67 s).
- The media test skips its recording half off macOS (recording uses VideoToolbox).

On Linux KVM API 30, `gps_fix_and_route_replay` and the media test (screenshot half) passed. The API 33 job failed in Stream A's `five_agent_tasks` ("UI never matched"). It passed on API 30, on main's last run, and this branch doesn't touch jidr-mcp, so it looks flaky.

## 2026-10-14: PR #3 review blockers

Branch `fix/b-review-blockers` (from `fix/b-player-emulator-waits`). Each fix has a regression test.
1. Snapshot path traversal: `resolve("..")` gave the id `..`, and `snapshot.delete` on a stopped device removed the whole AVD folder. Now `snapshot_id` never starts with `.`, `check_id` refuses anything but one plain folder name, `resolve` returns an error for bad ids (including a tag whose key is bad), and delete builds the path through `snapshot_dir`. Test: `names_never_reach_outside_the_snapshots_folder`.
2. AppleScript injection in the screenshot copy: the path went into the script text with only `"` escaped. Now it's passed as `argv` (`on run argv`). `media::default_path` also drops `\`, `"` and control characters. Tests: `the_image_path_is_an_argument_not_script`, `osascript_takes_the_path_from_argv` (runs osascript with a hostile path and checks it comes back verbatim and nothing ran), `default_paths_drop_quotes_backslashes_and_controls`.
3. A recording kept repeating the last frame until `max_seconds` after the device's frame stream ended. The reader (`pump`) now sets `stop` when the stream ends. Test: `the_recorder_stops_when_the_frame_stream_ends`.
4. GUI recording state was one global flag. It's now per device (`recording::Active`), and `player::sync` shows the active tab's device. Test: `recording_state_is_per_device`.

Not done in this pass (review follow-ups 5–9): unplayable MP4 after a push timeout, "Clear cache" also removing the daemon's downloads/, `media.screenshot` overwriting an existing path, logcat not restarting after reconnect, nits.

## 2026-10-15: snapshot.load waits for adb

On #7, `snapshot_round_trip_and_cross_root_refusal` failed on API 33 and 35: after `snapshot.load`, `adb shell` printed nothing for 30 s. LoadSnapshot returns before the guest's adbd connection is usable again, and the host adb server can keep the stale transport `offline`. `snapshot.load` now calls `device::wait_for_adb`: it probes `adb shell echo ok` (reports "Waiting for the device"), runs `adb reconnect offline` after 5 s without an answer (at most every 10 s), honours cancel, and returns `boot_timeout` with `adb get-state` and the last adb error after 60 s. The decision is `AdbWait::step`, with unit tests. The test helpers keep adb's stderr, and `eventually_shell` prints it plus `adb get-state` on timeout. Passes on API 31 arm64 (133 s).
