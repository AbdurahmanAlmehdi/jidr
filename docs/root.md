# Root modes

Jidr roots your own Android emulators. Root is a first-class device property
(`root_mode: none | adb | magisk`), switched with `jidr root set` and applied
per device — the shared SDK system images are never modified.

| Mode | Images | Mechanism | Verify |
|---|---|---|---|
| `none` | all | stock launch | `adb shell id` → uid 2000 |
| `adb` | `default`, `google_apis`, `aosp_atd` | `adb root` after every boot | `adb shell id` → uid 0 |
| `magisk` | all | patched per-device ramdisk, cold boot | `adb shell su -c id` → uid 0, `su -c 'magisk -v'` |

```bash
jidr root set <device> magisk   # one-click full root
jidr root set <device> adb      # shell root
jidr root set <device> none     # revert either to stock
jidr root verify <device>       # check the live uid against the mode
jidr root status <device>       # stored mode, health, Magisk version
```

> Play Integrity cannot pass on any emulator; Zygisk/DenyList only hides root
> from local checks.

## How Magisk mode works

`root.set magisk` runs the whole setup as one job, with progress and cancel:

1. **Fetch** the pinned Magisk release from the official GitHub release and
   check its SHA-256 (see [below](#magisk-licence)).
2. **Hash** the image's pristine `ramdisk.img`.
3. **Patch** a *per-device* copy on the host with `jidr-cpio`: `init` becomes
   `magiskinit`, the compressed payload goes under `overlay.d/sbin`, and the
   config is recorded in `.backup/.magisk`. The result is written to
   `<avd>/jidr/ramdisk.magisk-<ver>.img` atomically (temp file + rename).
4. **Cold boot** with `-ramdisk <patched>`.
5. **Install** the Magisk app (the release APK is also the manager app).
6. **Set up** `/data/adb/magisk`, then **reboot once** so `magiskd` enables su.
7. **Grant** the adb shell su automatically, then **verify**.

On `google_apis` (and `default` / `aosp_atd`) images this is fully zero-touch:
`adb root` is used to write Magisk's su-policy row for the shell, then adbd is
dropped back so the final check exercises Magisk's own `su`. Apps still get
Magisk's normal per-app prompt; the opt-in "auto-grant all" toggle is off by
default.

## Revert

`root.set none` (or `adb`) on a Magisk device clears `launch.ramdisk`, deletes
the patched file and its metadata, and cold-boots the stock ramdisk. `none` and
`adb` then behave exactly as they do on a fresh device. A failed Magisk setup
leaves the device in the same revertable state (health `failed`, patched file
still present).

## Re-patch

A patched ramdisk records the pristine SHA-256 and the Magisk version it was
built from. When either changes — an image update, or a newer pinned Magisk —
the next `root.set magisk` (or the boot hook) rebuilds the patched copy from the
pristine image. Patches never stack.

## Offline Magisk

To use Magisk without network access, import the official APK once:

```bash
jidr root set <device> magisk --magisk /path/to/Magisk-v30.7.apk
```

The CLI adds the APK to the download cache by SHA-256 and `root.set` uses that
cached copy (`magisk_sha256`) instead of downloading. The SHA-256 is still
checked, so a tampered or wrong file is rejected.

## Play Store images (beta)

`google_apis_playstore` images refuse `adb root`, so the shell grant cannot be
pre-approved. Jidr uses the guided tap from ADR-0002:

1. Full-root setup runs as above (patch, cold boot, app install, environment).
2. Jidr shows "Magisk will ask to grant root to Shell. Tap Grant." and runs
   `su -c id`, which raises Magisk's normal prompt.
3. It waits up to 60 s. Tap Grant, and the shell is granted root.
4. Denied or timed out, the device reports `awaiting_user_grant` with
   **Try again** and **Revert**.

There is no headless bypass on Play Store images; apps keep Magisk's per-app
prompt.

## Magisk licence

Magisk is **GPL-3.0-only**, so Jidr never bundles or redistributes it. It is
downloaded at runtime from
[topjohnwu/Magisk](https://github.com/topjohnwu/Magisk), its SHA-256 is checked
against the pin in `assets/magisk-pins.toml`, and the licence and source link
are shown before the first download. The same clean-room rule applies to the
patch itself: it reproduces Magisk's documented ramdisk layout without copying
its GPL code (see `docs/clean-room.md`).
