---
layout: ../../layouts/Post.astro
title: "How to root the Android emulator on an Apple Silicon Mac"
description: "Shell root with adb root, full root with Magisk patched into the emulator's ramdisk, and how to verify and revert both, on M-series Macs."
date: 2026-10-17
---

Google's Android Emulator runs arm64 system images natively on Apple Silicon, and it's fast. What it doesn't give you is root, and many security, testing and reverse-engineering workflows need it. There are two kinds of root you can have on an emulator, and they solve different problems.

## Shell root: `adb root`

Some system images let `adbd`, the daemon behind `adb shell`, restart as root:

```bash
adb root
adb shell id    # uid=0(root) gid=0(root) …
```

This works on the **Google APIs** (`google_apis`), **Default** (`default`) and **ATD** (`aosp_atd`) images. It doesn't work on **Google Play** (`google_apis_playstore`) images, which are production builds: `adb root` answers "adbd cannot run as root in production builds".

Shell root is enough to read any app's private data, change system settings or run `tcpdump`. But only your shell is root. Apps on the device can't get root, there's no `su` binary, and `adb root` has to be run again after every boot.

## Full root: Magisk in the ramdisk

For apps to request root (a root checker, a Frida server, an app under test that behaves differently when rooted), the device needs `su`. On an emulator the usual way is [Magisk](https://github.com/topjohnwu/Magisk), patched into the boot ramdisk.

The emulator loads the ramdisk from the system image's `ramdisk.img`, a compressed cpio archive whose `init` is the first process Android runs. Magisk's documented layout replaces that `init` with `magiskinit`, adds Magisk's payload under `overlay.d/sbin`, and records its config in `.backup/.magisk`. At boot, `magiskinit` sets up Magisk, then hands over to the real `init`.

Done by hand, it goes like this:

1. Copy the image's `ramdisk.img` somewhere of your own. Never patch the one in the SDK: every device created from that image would boot it.
2. Decompress and unpack the cpio, apply the changes above, repack and recompress it in the same format.
3. Cold boot the emulator with the patched copy: `emulator -avd <name> -ramdisk <patched.img> -no-snapshot-load`.
4. Install the Magisk app, which is also the release APK, and let it finish its environment setup. That needs one more reboot.
5. Grant `su` to the shell (Magisk prompts on the device) and check: `adb shell su -c id`.

Scripts such as rootAVD automate these steps. What they leave to you is keeping track of which ramdisk each device boots, and undoing the patch later.

## Doing it in one step with Jidr

[Jidr](https://github.com/AbdurahmanAlmehdi/jidr) is a free, open-source emulator manager for Apple Silicon Macs that treats root as a device setting:

```bash
jidr image install 35 --accept-licenses   # you accept Google's SDK licence
jidr device create Phone --image 35
jidr root set Phone adb       # shell root, reapplied after every boot
jidr root set Phone magisk    # full root
jidr root verify Phone        # checks the live uid against the mode
jidr root set Phone none      # back to stock
```

`root set Phone magisk` runs the steps above as one job you can follow or cancel:

- It downloads the pinned Magisk release from GitHub and checks its SHA-256. Jidr never bundles Magisk.
- It patches a **per-device** copy of the ramdisk, with its own cpio and compression code, and writes it atomically.
- It cold boots, installs the app, sets up `/data/adb/magisk` and reboots once.
- On images that allow `adb root`, it writes the shell's grant itself, then drops adbd back so the final `su -c id` goes through Magisk's own `su`. On Google Play images it asks you to tap **Grant** once.

Reverting deletes the patched copy and cold boots the stock ramdisk, so a device is never stuck half-rooted. When the image or the pinned Magisk version changes, the next run rebuilds the patch from the pristine ramdisk instead of patching on top of an old one.

The same works from the desktop app (a Root switch on each device) and from an AI agent over MCP.

## What root can't do

No emulator passes Play Integrity, rooted or not: it attests the device's hardware and boot state, which an emulator doesn't have. Magisk's DenyList hides root from local checks only. If your app depends on Play Integrity, test that part on a real device.

Snapshots taken with one root mode can't be restored under another, because the ramdisk differs. Jidr tags each snapshot with its root mode and refuses a mismatch instead of booting a broken device.

## Further reading

- [Root modes in the Jidr docs](https://github.com/AbdurahmanAlmehdi/jidr/blob/main/docs/root.md): the full Magisk flow, auto-grant, build properties, offline Magisk
- [Magisk's installation docs](https://topjohnwu.github.io/Magisk/install.html)
- [Jidr vs Genymotion Desktop](/vs/genymotion/)
