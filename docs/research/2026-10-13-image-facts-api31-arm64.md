# Image facts: `system-images;android-31;google_apis;arm64-v8a`

> Measured 2026-10-13 on a throwaway AVD (`jidr-proof`, Pixel 5 profile, 1080×2340) with emulator 35.6.11 on Apple Silicon. Answers `docs/gaps.md` G4 for this image; other API levels are still to be measured.

| Fact | Value | How measured |
|---|---|---|
| Android version / build type | 12 · `userdebug` · `ro.debuggable=1` | `getprop` |
| `adb root` | works (`uid=0(root) … context=u:r:su:s0`) | `adb root && adb shell id` |
| Kernel | `5.10.110-android12-9-00004-gb92ac325368e-ab8731800` | `uname -r` |
| cgroups | hybrid: v1 `blkio` at `/dev/blkio`, plus v1 `cpu`, `cpuset` and `memcg`; v2 unified at `/sys/fs/cgroup` with **no controllers enabled** | `mount \| grep cgroup`, `cgroup.controllers` |
| blkio files | only `blkio.bfq.*` stats; **no `throttle.*` files** | `ls /dev/blkio` |
| Kernel config | `CONFIG_BLK_DEV_THROTTLING` **not set**, `CONFIG_DM_DELAY` not set, `CONFIG_NET_SCH_NETEM=y`, `CONFIG_IFB=y`, `CONFIG_IOSCHED_BFQ=y`, `CONFIG_BLK_CGROUP_IOCOST=y` | `zcat /proc/config.gz` |
| Block devices | `/` on `/dev/block/dm-0` (ext4, ro); `/data` on `/dev/block/dm-4` (ext4, rw, major:minor `254:4`) | `/proc/mounts`, `/sys/block/dm-4/dev` |
| Network | default route via `wlan0` (`10.0.2.16` → gateway `10.0.2.2`); `eth0` also present; `ifb0` and `ifb1` available | `ip route get 8.8.8.8`, `tc qdisc show` |
| Traffic control | `/system/bin/tc` present; `netem` works on `wlan0` | see `docs/gaps.md` R2 |
| Telephony defaults | operator T-Mobile `310260`; radio tech HSPA (WCDMA) | `getprop gsm.*`, `dumpsys telephony.registry` |
| Wi-Fi | SSID `AndroidWifi`, RSSI fixed at −50 dBm | `dumpsys wifi` |
| `-prop` at launch | does **not** set `ro.product.model`, nor a new `persist.*` property | launched with `-prop ro.product.model=JidrOne -prop persist.jidr.test=1` |
| Host webcams | listed as `webcam0` and `webcam1` | `emulator -webcam-list` |
| Cold boot | ≈24 s to `Boot completed` (`-no-snapshot -no-boot-anim -no-window`) | emulator log |

## Display data point (early input for ADR-0001)

Measured with a Python gRPC client (`grpcio`, stubs generated from the SDK's `emulator_controller.proto`), streaming `streamScreenshot` with inline RGBA8888 while the screen scrolled continuously (adb swipes), each run 4 s:

| Requested `ImageFormat` | Delivered | Frame size | FPS | Dropped (`seq` gaps) |
|---|---|---|---|---|
| `width=540` only | 1080×2340 (**the downscale is ignored unless both width and height are set**) | 10.1 MB | 29.1 | 0 |
| `width=540, height=1170` | 540×1170 | 2.5 MB | 30.1 | 0 |
| `width=1080, height=2340` | 1080×2340 | 10.1 MB | 24.4 | 0 |

Takeaways for Phase 1:
- Always set **both** `width` and `height` to get server-side downscaling.
- Raise the client's gRPC max receive size: full frames are about 10 MB, and the default 4 MB limit fails with `RESOURCE_EXHAUSTED`.
- Inline RGBA already reaches ~30 fps at window size from Python. The swipe-driven screen changes may be the limit, so measure again in Rust with a steady animation.

## Caveats

- `system-images;android-36;google_apis;arm64-v8a` is present on disk but `avdmanager` rejects it as an invalid package path (incomplete package metadata). Reinstall it with `sdkmanager` before API 36 tests.
- No arm64 API 33/34/35 `google_apis` image is installed locally yet (API 35 is x86_64 only, API 34 is `default`).
