# Jidr research summary (2026-10-13)

## Product
Open-source (Apache-2.0) Genymotion Desktop alternative. Pure Rust, Slint/egui UI. macOS Apple Silicon first, then Linux/Windows.
Architecture: `jidr daemon` daemon owns all state; UI, `dsctl` CLI (gmtool-style), MCP server are clients over JSON-RPC (Unix socket / named pipe).
Backend trait; first backend = AOSP emulator (QEMU+HVF) driven headless via gRPC EmulatorController + telnet console + adb.
MVP: device manager + display; sensor widgets; CLI + MCP; recording & extras. HEADLINE: one-click rooted devices. Landing page (domain TBD — jidr.com taken; .ly/.dev/.io/.org appear free). Clean room: public Genymotion docs only.

## Genymotion (public docs)
- Device = hardware profile + OS image. Fields: name, CPU, RAM, heap, root toggle, navbar, virtual kbd, resolution/density, window style/skin, GLES version, build props (MODEL, BRAND, FINGERPRINT...).
- Lifecycle: start, quick boot, cold boot, edit, duplicate, factory reset, delete, logzip. Drag-drop: .apk install, .zip flash (/system), other files -> /sdcard/Download. Shared clipboard toggle.
- gmtool: admin start/stop/list/create/edit/details/stopall/factoryreset/delete/clone/hwprofiles/osimages/logzip; device install/flash/push/pull/logcatdump/logcatclear/adbconnect (selectors -n/--all/--start); config; exit codes 1..14.
- Genymotion Shell: battery, gps, rotation, android ids, diskio, network (wifi/mobile status, signal, mobile profile), phone/baseband, sensors.
- Widgets: battery, GPS(+GPX/KML routes PRO), capture (screenshot/record), identifiers, disk I/O, phone/SMS, GApps, clipboard, network profiles(PRO), baseband/SIM(PRO), camera/media injection(PRO), Android controls(PRO), dev tools incl root toggle(PRO), biometrics(PRO), motion sensors 3D(PRO), gamepad(PRO), device link.
- Root: Android ≤11 rooted by default; 12+ toggle via `persist.sys.root_access` 3/0; adb root; Superuser allow/deny prompt app.
- Player: right toolbar (vol, rotate, pixel-perfect, back/home/recent, power, widgets), bottom capture buttons, resizable, fullscreen, multitouch emulation via modifiers.
- Paid tiers $239.99–$479.99/yr; free tier gates CLI automation, network, camera, quick boot, root toggle.

## AOSP emulator (local SDK emulator 35.6.11, protos in ~/Library/Android/sdk/emulator/lib/)
- gRPC EmulatorController (experimental): streamScreenshot (RGBA/RGB/PNG, downscale, MMAP transport, seq/timestamps; no H.264), sendTouch/sendMouse/sendKey/injectWheel/streamInputEvent, setGps, setBattery, setPhysicalModel/setSensor/setPosture, sendPhone/sendSms/setPhoneNumber, clipboard get/set/stream, streamAudio/injectAudio, setDisplayConfigurations, getStatus, setVmState, streamLogcat, sendFingerprint. SnapshotService: list/save/load/delete/pull/push.
- Auth: -grpc <port>, -grpc-use-token (bearer, from discovery file ~/Library/Caches/TemporaryItems/avd/running/pid_N.ini) or JWT.
- Console-only: network speed/delay, power ac/health, gsm signal/data/voice, screenrecord (webm), redir, proxy.
- Flags: -no-window, -no-snapshot(-load/-save), -read-only (multi-instance), -port, -wipe-data, -writable-system, -ramdisk, -camera-back webcamN/emulated/virtualscene.
- AVD: sdkmanager/avdmanager; config.ini hw.lcd.*, hw.ramSize, hw.cpu.ncore, skin; ports 5554+ even (adb scans to 5585 ≈16 instances); cloning AVD dirs is uncertain.
- Images: default/google_apis allow adb root; playstore do not. Apple Silicon needs arm64 images (availability varies per API).
- Recording: adb screenrecord (180s cap, no audio) vs console webm vs encoding streamed frames ourselves.
- Crates: tonic 0.14 / prost 0.14, slint 1.18 (SharedPixelBuffer per frame via invoke_from_event_loop; unstable wgpu texture import), egui 0.36, adb_client 3.2 (wrap adb binary instead), rmcp 3.5 (official MCP SDK), jsonrpsee 0.26 (no unix-socket server; custom loop or NDJSON over UnixListener), tokio 1.53.
- Licensing: never bundle emulator/system images (SDK license §3.4, GPL bits in emulator). Download via sdkmanager on user's machine after explicit license consent. No Google/Android logos.

## Root research
- Modes: None / adb-root (default, google_apis, atd images; optional -writable-system + remount) / Magisk (ramdisk patch).
- Magisk: never modify shared SDK ramdisk.img; write per-AVD patched copy, boot with `-ramdisk <patched> -no-snapshot-load` (+ `-feature -SystemAsRoot` on API 28). Download Magisk APK at runtime from official GitHub releases, pin SHA-256 (GPL-3.0 — don't bundle, don't copy rootAVD/Magisk scripts; avdroot is MIT prior art).
- Install Magisk app, complete env setup (+1 reboot), grant shell su (headless on debuggable images via adb root + magisk --sqlite; Play Store images need a UI tap — open problem).
- Verify: `adb shell su -c id` → uid=0. Re-patch when pristine ramdisk hash changes or Magisk upgrades. Snapshots tagged with root mode.
- Play Integrity can never pass on an emulator — say so in UI. KernelSU/userdebug-builds not recommended.
- API matrix: 29–36 core; 28 beta; 25–27 verify arm64 availability; 16k-page images untested.

## Name risk
- github.com/jidr free; two 0-star hobby repos named Jidr. No company/product found.
- "DROID" is a Lucasfilm trademark with enforcement history (Twidroid, Addroid). Check USPTO/WIPO/EUIPO class 9/42; keep a fallback name.
