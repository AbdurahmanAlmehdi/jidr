# Parity with Genymotion Desktop

Hand-edited. A row is ✅ only with evidence: a test name or a `file:line`. Report two numbers together, never breadth alone:

- **Breadth**: ✅ rows / all rows.
- **Ready for real work**: a judged estimate, with reasons.

Status: ✅ done with evidence · 🚧 in progress · ⬜ not started · ⏭️ post-MVP · 🔎 needs research (D11)

| Feature | Genymotion tier | Jidr status | Evidence |
|---|---|---|---|
| Create / edit / clone / delete device | Free | ⬜ | |
| Factory reset | Free | ⬜ | |
| Start / stop, cold boot | Free | ⬜ | |
| Quick boot | Paid | ⬜ | |
| Device list and details | Free | ⬜ | |
| Log zip | Free | ⬜ | |
| CLI automation (`gmtool admin/device`) | Paid | ⬜ | |
| Install APK / push / pull / logcat | Free | ⬜ | |
| Zip flash | Free | 🔎 | |
| Root (Android ≤ 11) | Free | ⬜ | |
| Root toggle (Android 12+) | Paid | ⬜ | |
| Magisk root | — (Jidr only) | ⬜ | |
| Battery widget | Free | ✅ | `widgets_emulator::widgets_core_commands` (`dumpsys battery`) |
| GPS widget | Free | ✅ | `gps_emulator::gps_fix_and_route_replay` (`dumpsys location`) |
| GPS routes (GPX/KML) | Paid | ✅ | `gps_emulator::gps_fix_and_route_replay` (3-point GPX in order) |
| Rotation | Free | ✅ | `widgets_core_commands` (`dumpsys window` mRotation) |
| Motion sensors 3D | Paid | 🚧 | `sensor.set`/`sensor.get` (crates/jidr-widgets/src/sensors.rs); environment sensors tested, motion presets not |
| Phone call / SMS | Free | ✅ | `widgets_core_commands` (`mCallState`, SMS inbox) |
| Baseband gsm signal/data/voice | Paid | ✅ | `widgets_core_commands` (signal bars via WCDMA ss); data/voice via `phone.gsm` |
| Baseband operator & SIM identity | Paid | 🔎 | |
| Network Wi-Fi / mobile toggles, speed/delay profiles | Paid | ✅ | `widgets_core_commands` (`settings get`, `adb emu network status`) |
| Network packet loss / bad DNS | Paid | 🔎 | |
| Identifiers (Android ID, IMEI) | Free | ⏭️ 🔎 | |
| Build props | Free | 🔎 | |
| Disk I/O limit | Free | 🔎 | |
| Camera / media injection | Paid | 🚧 🔎 | host webcam as back camera at next start (`camera.set`, R7); image/video files 🔎 |
| Microphone / audio injection | Paid | ⏭️ | |
| Biometrics | Paid | 🚧 | fingerprint touch/remove (`widgets_core_commands`, FingerprintHal); scenarios ⏭️ 🔎 |
| Android controls (D-pad, volume, media keys) | Paid | ✅ | `widgets_core_commands` (Home → launcher focus, volume panel) |
| Dev tools (show taps, pointer location, layout bounds, animations) | Paid | ✅ | `widgets_core_commands` (`settings get`, `getprop debug.layout`) |
| Gamepad | Paid | ⏭️ 🔎 | |
| Device Link | Free | ⏭️ 🔎 | |
| Clipboard sharing | Free | 🚧 | `clipboard.get/set` tested; two-way sharing toggle in the player (crates/jidr-ui/src/player/session.rs) not tested |
| Drag-drop APK / files | Free | ✅ | manual check (installed and opened `ly.jidr.hello`) |
| Screenshot | Free | ✅ | `media_emulator::media_screenshot_and_five_minute_recording` |
| Screen recording | Free | ✅ | `media_screenshot_and_five_minute_recording` (ffprobe h264, 300 s, 9000 frames) |
| Snapshot manager | — (Jidr only) | ✅ | `snapshot_emulator::snapshot_round_trip_and_cross_root_refusal` |
| Shared folders | — (Jidr only) | ⏭️ | |
| Multitouch via modifiers | Free | 🚧 | ⌥ pinch / ⇧⌥ pan (crates/jidr-ui/src/player/input.rs, unit tests only) |
| Pixel-perfect, fullscreen, resize | Free | 🚧 | pixel-perfect and resize (golden `player-pixel-perfect`); fullscreen not done |
| MCP server for AI agents | — (Jidr only) | ⬜ | |
| Shell REPL (Genymotion Shell) | Free | ⏭️ | |

**Not yet recounted after the MVP merges:** some rows (device management, CLI, MCP, root) still show ⬜ or 🚧 for features that have shipped. The table is being brought up to date with evidence.
