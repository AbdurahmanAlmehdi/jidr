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
