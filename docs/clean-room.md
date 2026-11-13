# Clean-room policy

Jidr re-creates the *behaviour* of Genymotion Desktop's public feature set. It must never derive from Genymotion's code, or from GPL code we don't want to inherit.

## Allowed sources

| Source | Use |
|---|---|
| Genymotion public docs (docs.genymotion.com, genymotion.com/pricing, /features) | Feature lists, user-visible behaviour, CLI command names, exit-code semantics |
| AOSP emulator public protos (`emulator_controller.proto`, `snapshot_service.proto`, shipped in the SDK) | gRPC API; we generate code from them (Apache-2.0) |
| developer.android.com emulator docs (command line, console, AVD management) | Flags, console commands, config.ini keys |
| Android SDK tools (`sdkmanager`, `avdmanager`, `adb`, `emulator`), run on the user's machine | Observed behaviour, outputs |
| Magisk and rootAVD scripts (GPL-3.0) | **Read only**, to learn the documented ramdisk layout. Never copy or translate code. |
| avdroot (MIT) | Prior art; may be ported with attribution in `NOTICE` |
| Public specs (cpio newc, gzip, lz4 legacy, xz) | Format implementations |

## Forbidden

- Decompiling, disassembling, string-dumping or otherwise inspecting Genymotion binaries, installers, images or network traffic.
- Copying or line-by-line translating GPL code (Magisk, rootAVD) into Apache-2.0 crates.
- Bundling or redistributing the Android SDK, emulator, system images or Magisk.
- Google, Android or Genymotion logos anywhere in the product or site.

## Log

Record any non-obvious source you used, so provenance can be audited later.

| Date | Stream | Source | What it informed |
|---|---|---|---|
| 2026-10-13 | A | SDK `emulator/lib/emulator_controller.proto`, `snapshot_service.proto` (emulator 35.6.11) | gRPC bindings (`jidr-proto`) |
| 2026-10-13 | C | `man 5 cpio` (newc), the Linux kernel's initramfs buffer-format doc, RFC 1952 (gzip), the lz4 frame-format doc (legacy frame), the `.xz` format spec; the local SDK `ramdisk.img` files, read-only, decompressed with the `lz4`/`gzip` CLIs to observe the layout | `jidr-cpio` (Phase 9b) |
| 2026-10-14 | C | Magisk's `scripts/boot_patch.sh` (read-only, GPL) and avdroot's `internal/{magisk,cpio,adb}` (MIT) — the documented ramdisk entries (`init` → `magiskinit`, `overlay.d/sbin/{magisk,stub,init-ld}.xz`, `.backup/.magisk`), the payload/ABI layout inside the APK, the `/data/adb/magisk` environment, and the su-policy grant (`magisk --sqlite`). avdroot's algorithm is ported with attribution in `NOTICE`; no GPL code was copied. | `jidr-root` Magisk mode (Phase 9d) |
| 2026-10-15 | C | Magisk v30.7 `app/core` `Config.kt`, `SuRequestHandler.kt`, `PreferenceConfig.kt` and `native/src/core/su/db.rs` (read-only, GPL) — learned that the superuser "automatic response" is the SharedPreferences key `su_auto_response` (values `0` prompt / `1` deny / `2` allow), not a `magisk --sqlite` `settings` row; the native `uid_granted_root` only consults per-uid `policies`. No GPL code was copied. | `jidr-root` `root.auto_grant` (G15) |
| 2026-11-02 | B | `emulator_controller.proto` comments on `Touch.x`/`y` ("physical location on the screen") and `ImageFormat.rotation`; a temp tablet AVD with `pointer_location` on and `dumpsys input`, read over `adb` | Player touches are sent in unrotated panel pixels, mapped through the frame's rotation (#51) |
| 2026-11-04 | C | The Android emulator's public command-line docs (`-writable-system`) and AOSP's public adb and `fs_mgr` overlayfs docs (`adb remount`, `adb enable-verity`, the reboot the first remount asks for) | `jidr-root` `root.writable_system` (#25) |
| 2026-11-04 | B | `emulator_controller.proto` comments on `injectAudio`, `AudioPacket` and `AudioFormat` (U8/S16, mono/stereo, the ~300 ms buffer, format honoured on the first packet only, `MODE_REAL_TIME` experimental, `INVALID_ARGUMENT` / `FAILED_PRECONDITION`); Microsoft's public RIFF/WAVE and `WAVEFORMATEXTENSIBLE` format docs | `audio.inject` / `audio.stop` and the WAV parser in `jidr-backend` (#70) |
| 2026-11-12 | C | The Linux `tc(8)` and `tc-netem(8)` man pages, `ip-route(8)`, `unix(7)` (abstract socket names) and `socket(7)` (`SO_PEERCRED`), Android's public `adb` docs (`adb forward … localabstract:`); the emulator's own `tc qdisc show` and `ip route show table all` output, read over `adb` on a throwaway AVD | The on-device helper (ADR-0003): netem packet loss, picking interfaces by their default routes, the abstract socket and its peer-uid check |
| 2026-11-03 | C | AOSP `packages/modules/Wifi` `WifiConfigManager.java` (`saveToStore` → `BUFFERED_WRITE_ALARM_INTERVAL_MS` = 10 s) and `WifiServiceImpl.java` (`ACTION_SHUTDOWN` → `writeDataToStorage`), `device/generic/goldfish` `ProvisionActivity.java` (Apache-2.0, read); logcat and `cmd wifi` on a temp API 35 AVD | Root-flow cold boots shut Android down first, so the first boot's `AndroidWifi` network is kept (#78) |
