# Parity with Genymotion Desktop

Hand-edited. A row is ✅ only with evidence: a test name or a `file:line`. Report two numbers together, never breadth alone:

- **Breadth**: ✅ rows / all rows.
- **Ready for real work**: a judged estimate, with reasons.

Status: ✅ done with evidence · 🚧 partly done · ⬜ not started · ⏭️ post-MVP (linked issue) · ❌ won't do · 🔎 needs research (D11)

Tests named `*_emulator` run on a real emulator: on Linux KVM in CI (API 30, 33, 35) and on Apple Silicon in `cargo xtask e2e-mac`. The others run offline against `TestDaemon` (fake backend and SDK) or fakes.

| Feature | Genymotion tier | Jidr status | Evidence |
|---|---|---|---|
| Create / edit / clone / delete device | Free | ✅ | `crates/jidr-sdk/tests/avd_emulator.rs:48` `create_clone_boot_delete`; `crates/jidr-daemon/tests/conformance.rs:85` `device_lifecycle` (create, edit, clone, delete) |
| Factory reset | Free | ✅ | `conformance.rs:85` `device_lifecycle` (`device.reset`); `apps/jidr/tests/cli.rs:9` `device_lifecycle_json` (`device reset`) |
| Start / stop, cold boot | Free | ✅ | `crates/jidr-backend/tests/launch_emulator.rs:16` `boot_battery_stop_ramdisk_quickboot` (boot, stop, cold boot); `scripts/e2e-cli.sh` (start, stop) |
| Quick boot | Paid | ✅ | `launch_emulator.rs:16` `boot_battery_stop_ramdisk_quickboot`; `conformance.rs:288` `settings_drive_create_and_quick_boot`. Resume takes about 13.5 s on the dev Mac against a 5 s target |
| Device list and details | Free | ✅ | `conformance.rs:85` `device_lifecycle` (`device.list`, `device.details`); `cli.rs:96` `discovery_commands` |
| Log zip | Free | ✅ | `apps/jidr/tests/maintenance.rs:49` `device_log_bundle` (`jidr device logs --zip`) |
| CLI automation (`gmtool admin/device`) | Paid | ✅ | `cli.rs:9` `device_lifecycle_json`, `cli.rs:44` `exit_codes_are_error_codes`; `scripts/e2e-cli.sh` (create → start → install → logcat → stop → delete, JSON throughout) |
| Install APK / push / pull / logcat | Free | ✅ | `scripts/e2e-cli.sh` (install, logcat); `crates/jidr-adb/tests/fake_adb.rs:139` `push_pull`; `crates/jidr-daemon/tests/jobs_events.rs:112` `logcat_follow_streams_lines` |
| Zip flash | Free | 🚧 | A dropped zip installs as a Magisk module on Full-root devices and is pushed to `/sdcard/Download` otherwise (D35): `crates/jidr-ui/src/player/drop.rs:79` `drops_route_by_type_and_root`, `crates/jidr-root/tests/magisk_mode.rs:150` `module_install_refuses_non_magisk_devices`. No emulator test of a module install; image flashing isn't offered |
| Root (Android ≤ 11) | Free | ✅ | `crates/jidr-root/tests/adb_emulator.rs:47` `adb_root_cycle_none_adb_none` (API 30; API 31 in `e2e-mac`) |
| Root toggle (Android 12+) | Paid | ✅ | `adb_emulator.rs:47` `adb_root_cycle_none_adb_none` (API 33 and 35 in CI); `crates/jidr-root/tests/adb_mode.rs` (live switch, cold-boot apply, verify) |
| Magisk root | — (Jidr only) | ✅ | `crates/jidr-root/tests/magisk_emulator.rs:49` `magisk_root_cycle_none_magisk_none`; `scripts/e2e-root.sh` (none → adb → magisk → none, `su -c id`) |
| Full root on Play Store images (guided beta) | — (Jidr only) | 🚧 | Guided grant (ADR-0002): `crates/jidr-root/src/magisk/grant.rs:124` `grant_shell_guided`, unit-tested offline; not run end to end on a Play Store image. Unattended options: ⏭️ [#24](https://github.com/AbdurahmanAlmehdi/jidr/issues/24) |
| Battery widget | Free | ✅ | `crates/jidr-widgets/tests/widgets_emulator.rs` `widgets_core_commands` (`dumpsys battery`) |
| GPS widget | Free | ✅ | `crates/jidr-widgets/tests/gps_emulator.rs` `gps_fix_and_route_replay` (`dumpsys location`) |
| GPS routes (GPX/KML) | Paid | ✅ | `gps_emulator.rs` `gps_fix_and_route_replay` (3-point GPX in order) |
| Rotation | Free | ✅ | `widgets_core_commands` (`sensor.rotate`, `dumpsys window` rotation) |
| Motion sensors 3D | Paid | 🚧 | `sensor.set` / `sensor.get` (`crates/jidr-widgets/src/sensors.rs`); rotation and the light sensor are tested in `widgets_core_commands`, accelerometer and gyroscope presets aren't |
| Phone call / SMS | Free | ✅ | `widgets_core_commands` (`mCallState` ringing and idle, SMS in the inbox) |
| Baseband gsm signal/data/voice | Paid | ✅ | `widgets_core_commands` (signal level 0 and ≥ 3 on any radio via `phone.gsm`); data and voice registration via `phone.gsm` |
| Baseband operator & SIM identity | Paid | ⏭️ | [#16](https://github.com/AbdurahmanAlmehdi/jidr/issues/16); the phone number alone works natively |
| Network Wi-Fi / mobile toggles, speed/delay profiles | Paid | ✅ | `widgets_core_commands` (Wi-Fi via `settings`, mobile data via the emulated radio and `network.get`, speed via `adb emu network status`) |
| Network packet loss / bad DNS | Paid | ⏭️ | [#17](https://github.com/AbdurahmanAlmehdi/jidr/issues/17) |
| Identifiers (Android ID, IMEI) | Free | ⏭️ | [#19](https://github.com/AbdurahmanAlmehdi/jidr/issues/19) |
| Build props | Free | ✅ | `crates/jidr-root/tests/props_emulator.rs:61` `props_spoof_after_cold_boot_and_stock_after_revert` (Full-root devices only) |
| Disk I/O limit | Free | ⏭️ | [#20](https://github.com/AbdurahmanAlmehdi/jidr/issues/20) |
| Camera / media injection | Paid | 🚧 | Host webcam as a camera at the next start: `widgets_core_commands` (`camera.set` → `-camera-back webcam0` in the launch args). Image and video files: ⏭️ [#21](https://github.com/AbdurahmanAlmehdi/jidr/issues/21) |
| Microphone / audio injection | Paid | ⏭️ | Not built; the emulator exposes `injectAudio` over gRPC. No issue yet |
| Biometrics | Paid | 🚧 | Fingerprint touch and remove: `widgets_core_commands` (the fingerprint HAL is checked on HIDL and AIDL images). Dirty / partial / insufficient / too fast: ⏭️ [#22](https://github.com/AbdurahmanAlmehdi/jidr/issues/22) |
| Android controls (D-pad, volume, media keys) | Paid | ✅ | `widgets_core_commands` (Home → launcher focus, volume panel) |
| Dev tools (show taps, pointer location, layout bounds, animations) | Paid | ✅ | `widgets_core_commands` (`dev.set`, read back with `settings get` and `getprop debug.layout`) |
| Gamepad | Paid | ⏭️ | [#23](https://github.com/AbdurahmanAlmehdi/jidr/issues/23) |
| Device Link | Free | ❌ | Rejected (D37) |
| Clipboard sharing | Free | 🚧 | `clipboard.get` / `clipboard.set` tested in `widgets_core_commands`; the two-way sharing toggle in the player (`crates/jidr-ui/src/player/session.rs`) isn't |
| Drag-drop APK / files | Free | ✅ | `crates/jidr-ui/src/player/drop.rs:79` `drops_route_by_type_and_root`; manual check (a dropped APK installed and opened) |
| Screenshot | Free | ✅ | `crates/jidr-widgets/tests/media_emulator.rs` `media_screenshot_and_five_minute_recording` |
| Screen recording | Free | ✅ | `media_screenshot_and_five_minute_recording` (ffprobe: h264, 300 s, 9000 frames). Audio: ⏭️ [#28](https://github.com/AbdurahmanAlmehdi/jidr/issues/28) |
| Snapshot manager | — (Jidr only) | ✅ | `crates/jidr-widgets/tests/snapshot_emulator.rs` `snapshot_round_trip_and_cross_root_refusal` |
| Shared folders | — (Jidr only) | ⏭️ | [#27](https://github.com/AbdurahmanAlmehdi/jidr/issues/27) |
| Multitouch via modifiers | Free | 🚧 | ⌥ pinch and ⇧⌥ pan: `crates/jidr-ui/src/player/input.rs` `gestures_add_a_second_finger` (unit test only) |
| Pixel-perfect, fullscreen, resize | Free | 🚧 | Pixel-perfect and resize (golden `player-pixel-perfect`); fullscreen not built |
| MCP server for AI agents | — (Jidr only) | ✅ | `crates/jidr-mcp/tests/mcp.rs:25` `tools_are_listed_with_annotations`, `mcp.rs:163` `destructive_policy_d5`; `crates/jidr-mcp/tests/agent_tasks.rs:175` `five_agent_tasks` (real emulator) |
| Shell REPL (Genymotion Shell) | Free | ⏭️ | Future consideration; the CLI covers scripting today. No issue yet |

**Breadth:** 26 / 43 ✅ (60%). Of the other 17: 8 🚧, 8 ⏭️, 1 ❌. Five rows are Jidr-only (not in Genymotion); three of them are ✅.

**Ready for real work: about 70% for the core use case, lower for Genymotion power users.**

- **Core use case:** create a device on Apple Silicon, root it, test an app, and automate it from a CLI, CI or an AI agent. Every step has an emulator test, and the whole path passes `cargo xtask e2e-mac` (345 emulator tests plus the CLI, root and SDK-untouched scripts) and CI on API 30, 33 and 35.
- **What holds it back from "ready":**
  - There's no signed release yet; v0.1.0 is in preparation.
  - Quick boot is slower than its target.
  - A few emulator tests are still flaky on shared CI runners.
  - Several paid-tier niches are partial or post-MVP: motion presets, file-based camera, biometric scenarios, network loss and DNS, identifiers, disk limits, gamepad.
  - Fullscreen and clipboard sharing in the player aren't finished.
