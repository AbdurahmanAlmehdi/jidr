# Jidr (جذر): product brief for designers

> The source of truth for design work. The full technical plan is in `docs/plan/2026-10-13-feat-jidr-emulator-suite-plan.md`; read §"Decisions log" there if anything here seems ambiguous.

## 1. What Jidr is

Jidr (Arabic جذر, *JID-er*, "root") is a **free, open-source Android emulator for the Mac**. It is a desktop app, plus a command-line tool and an AI-agent (MCP) server, that creates and runs virtual Android phones and tablets.

It does what Genymotion Desktop does (a polished device manager, a device window and sensor controls) and adds four things:

1. **One-click rooted devices.** You pick a root mode when you create a device: *None*, *Shell root (adb)* or *Full root (Magisk)*. Jidr does the rest and shows a verified "Rooted" badge. This is the headline feature.
2. **Everything free.** Genymotion's paid features (CLI automation, network profiles, GPS route replay, quick boot, root toggle) cost nothing here.
3. **Agent-native.** Every button is also a CLI command and an MCP tool, so an AI agent can drive a device: screenshot, tap, type, read logs.
4. **Snapshots done right.** Save and restore device states, each tagged with its root mode.

Under the hood it runs Google's open-source Android emulator headless. Jidr's own window shows the screen. We never ship Google's binaries: Jidr downloads them on the user's machine after they accept Google's license.

## 2. Who uses it

| Persona | Wants | Signature moment |
|---|---|---|
| **Sara, mobile developer** | A fast, good-looking emulator that's free; sensors for testing (GPS, battery, network) | Drags an APK onto the device; it installs |
| **Karim, security researcher** | Rooted devices with Magisk, Play Store images, snapshots to roll back | Clicks *Full root*; 60 s later, "Rooted · Magisk 30.7 ✓" |
| **CI pipeline** | Headless create → boot → test → destroy, JSON output | Never sees the UI (CLI only) |
| **AI agent (via MCP)** | Screenshot, tap, read UI tree, logcat | Never sees the UI, but users watch the device window while an agent drives it |

Users are technical, on macOS (Apple Silicon), and often keep the app open all day beside their IDE. They value density, keyboard shortcuts and speed, but the first-run experience must be friendly for someone who has never installed the Android SDK.

## 3. Surfaces

1. **Launcher window**: the device library. Lists devices, their status, Android version, root mode, disk usage; create, start, stop, clone, delete; jobs in progress (downloads, root setup).
2. **First-run setup**: detects whether an Android SDK exists; shows Google's SDK license (must be scrolled and explicitly accepted); downloads the emulator and a first system image with progress.
3. **Create / edit device**: hardware profile, Android version (system image), root mode, RAM, CPU cores, resolution and density, name.
4. **Device window (player)**: the live Android screen (the hero), a vertical toolbar (volume, rotate, back, home, recents, power, screenshot, record, pixel-perfect) and a **widget panel** that opens beside the screen.
5. **Widgets**: Battery · GPS (map, coordinates, GPX/KML route replay) · Rotation and motion sensors · Phone and SMS · Network (Wi-Fi/mobile on/off, speed/delay profiles) · Identifiers · Android controls (D-pad, media keys) · Developer tools (root, show taps, layout bounds, animations) · Biometrics (fingerprint touch) · Clipboard · Microphone input · Snapshots · Logcat.
6. **Root setup flow**: a short, honest progress sequence (download Magisk → patch → reboot → install → verify), with a clear success badge, a clear failure state, and one-click revert.
7. **Settings**: SDK location and disk usage, cache, MCP setup ("Add Jidr to Claude"), map tile source, default root mode, about and licenses.
8. **Landing page (jidr.ly)**: a pre-launch "coming soon" page, then the full marketing site: hero, root showcase, "Genymotion's paid features, free", agent/MCP section, CLI snippet, download, sponsors, Arabic version.

## 4. Non-negotiables

- **The Android screen is the hero** of the device window. Chrome around it is quiet and narrow; nothing overlaps the screen.
- **Root state is unmistakable** wherever a device appears: label + icon, never colour alone. "Verified" vs "Unverified" vs "Failed" must be distinct.
- **Status is honest**: Booting, Running, Stopped, Crashed, Busy (another client is using it) are always visible on device rows.
- **Keyboard-first**: every action has a shortcut or command palette (⌘K) entry.
- **Native macOS feel**: traffic lights, standard menus, standard sheet and popover behaviour; dark mode first-class.
- **Buildable in Slint** (Rust GUI toolkit): solid fills, borders, rounded rects, simple shadows, images, paths; no heavy blur/vibrancy dependencies, no web views.
- **No Google, Android or Genymotion logos or trademarks** in the UI or site (Google SDK license §3.7). Use our own device silhouettes.
- **Honesty about limits**: e.g. "Rooted emulators can't pass Play Integrity" is stated plainly where relevant.
- **RTL-ready**: English-only at launch, but layouts must mirror cleanly for Arabic later.
- **Accessible**: text contrast ≥ 4.5:1, focus rings visible, nothing conveyed by colour alone.

## 5. Statuses (user-facing labels)

| Kind | States (label shown to user) |
|---|---|
| Device | Stopped · Starting… · Booting Android… · Running · Paused · Stopping… · Crashed · Busy (in use by CLI/agent) |
| Root | Not rooted · Shell root (adb) · Full root · Magisk 30.7 · Setting up root… · Rooted ✓ (verified) · Root unverified · Root failed: details · Beta (Play Store image) |
| Job | Queued · Downloading 1.4 / 2.1 GB · Patching · Rebooting · Verifying · Done · Failed · Cancelled |
| Snapshot | Quick boot (auto) · Named snapshot · Taken with: Full root / Shell root / Not rooted |
| SDK | Not installed · Needs license acceptance · Installing · Ready · Update available |

## 6. What the daemon really returns (design within this data)

- **Device**: id, name, profile name, screen W×H and density, Android version + API level (e.g. "Android 15 · API 35"), image tag (Google APIs / Play Store / AOSP), status, root mode + root health + Magisk version + last verified time, RAM, CPU cores, disk usage (GB), adb serial (`emulator-5556`), uptime, last started.
- **Job**: id, kind, device, progress 0–1, bytes done/total, state, error message.
- **Snapshot**: name, created at, size, root mode tag, thumbnail (PNG).
- **Widgets**: battery level and charging state; GPS lat/long/alt/accuracy/bearing; rotation angle; network type + speed/delay profile; Wi-Fi/mobile on/off; clipboard text; Android ID.
- **Logcat**: timestamp, level (V/D/I/W/E/F), tag, pid, message.
- **Not available** (don't design as if they exist; mark as optional ideas if you want them): live CPU/RAM graphs per device, per-app network traffic, cloud devices, accounts or sign-in, team sharing.

## 7. Sample data (use this so screens feel real and agree)

| Device | Profile | Android | Image | Root | Status | Disk |
|---|---|---|---|---|---|---|
| **Pixel-ish 6.1** | Phone 6.1″ FHD+ (1080×2400, 420 dpi) | Android 15 · API 35 | Google APIs | Rooted ✓ · Magisk 30.7 | Running · 2 h 14 m | 6.8 GB |
| **Tablet 11** | Tablet 11″ (1600×2560, 320 dpi) | Android 14 · API 34 | Google APIs | Shell root (adb) | Stopped | 4.1 GB |
| **Play Store test** | Phone 6.1″ FHD+ | Android 13 · API 33 | Play Store | Full root · Beta | Booting Android… 62% | 5.3 GB |
| **CI-runner-03** | Small phone 5″ (720×1280) | Android 11 · API 30 | AOSP | Not rooted | Busy · in use by CLI | 3.0 GB |

- Job: "Downloading Android 16 (API 36) image · 1.4 / 2.1 GB · 2 min left".
- GPS: Tripoli Martyrs' Square, 32.8954, 13.1802; route replay `tripoli-airport-run.gpx` at 2×.
- Battery: 64%, discharging.
- Snapshots on Pixel-ish 6.1: "Clean install" (Full root, 2.1 GB), "Logged in test account" (Full root, 2.4 GB), "Quick boot" (auto).
- Logcat lines: `I/ActivityManager: Start proc 4821:com.example.shop/u0a190`, `W/Magisk: su: request from uid 10190 (com.example.shop)`, `E/AndroidRuntime: FATAL EXCEPTION: main`.
- CLI snippet: `jidr device create "Pixel-ish 6.1" --api 35 --root magisk --wait-boot`.

## 8. Brand

The app icon and symbol are done: see `assets/brand/README.md` (symbol = a root growing into a phone outline; brand blue `#2A88FA → #1267E0`, lilac `#B9A6F7` for the Arabic wordmark).

## 9. Tone

Confident, technical, warm. Short labels. Say what happens ("Reboots the device to finish root setup (about 40 s)"). No hype, no exclamation marks, no emoji in the UI.
