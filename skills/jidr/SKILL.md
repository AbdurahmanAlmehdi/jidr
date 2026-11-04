---
name: jidr
description: "Run, watch and drive Android emulators with Jidr (CLI `jidr`, the Jidr desktop app, MCP server `jidr mcp`). Use when the user mentions Jidr or a Jidr device, or asks for an Android phone or tablet emulator to run something on, an emulator screenshot, logcat, rotation, snapshots, or starting and stopping a device. For testing an app's flows on a device, also load jidr-app-testing."
---

# Jidr

Jidr (جذر) runs Android emulators, optionally rooted. A daemon owns every device. The desktop app, the `jidr` CLI and the MCP server (`jidr mcp`) are thin clients over the same protocol, so a device started from one shows up in all of them. Today Jidr ships for macOS on Apple Silicon (arm64 images). Docs: https://github.com/AbdurahmanAlmehdi/jidr (`docs/cli.md`, `docs/mcp.md`, `docs/control-protocol.md`).

If `jidr` isn't on the `PATH`, ask the user to install Jidr (https://jidr.ly, or `brew install --cask abdurahmanalmehdi/tap/jidr`). The macOS app also carries the CLI at `Jidr.app/Contents/Helpers/jidr`.

**Prefer the CLI.** It covers every command, needs no setup and works in scripts. `jidr call <method> '<json>'` reaches any protocol command that has no subcommand. A Jidr device is not an Android Studio AVD: `emulator -list-avds` doesn't list it. A `.jidr` file is a script for `jidr shell` (one `jidr` command per line, without the `jidr`); `jidr shell file.jidr` runs it and stops at the first failing line. Interactively, prefer one-shot `jidr` commands over the shell.

## Find or create a device

```bash
jidr device list                 # NAME, STATUS, IMAGE, ROOT, SERIAL (the adb serial while running)
jidr --json device list          # the same as JSON
jidr profile list                # phone-6.1-fhd, phone-6.7-qhd, small-phone-5, tablet-11, agent-lean
jidr image list --installed      # images already downloaded (works offline)
```

Reuse a stopped device whose form factor fits the app. Otherwise create one from an installed image, so nothing has to download:

```bash
jidr -q device create Phone --image 35 --profile phone-6.1-fhd --agent-owned
jidr -q device start Phone --timeout 300    # waits for boot, prints the line with the adb serial
```

- `--image` takes `35`, `35:google_apis`, `35:google_apis_playstore` or a full `system-images;…` package.
- `--agent-owned` marks the device as yours, so you (or the MCP server) may delete it later (D5).
- Installing a new image downloads 1–2 GB and needs the user to accept Google's SDK license. Ask before `jidr image install`, and never pass `--accept-licenses` for them.

## Let the user watch

The daemon runs devices headless. To see the screen, the user opens the Jidr app and then the device's player window (on macOS: `open -a Jidr`). Tell them that's where to look.

## Look, act and read logs

```bash
jidr --json call media.screenshot '{"device":"Phone"}'        # result.path is the PNG; read it to see the screen
jidr call sensor.rotate '{"device":"Phone","angle":90}'       # 0, 90, 180 or 270
jidr call key.press '{"device":"Phone","key":"back"}'         # back, home, recents, enter, power, volume_up…
jidr call app.launch '{"device":"Phone","package":"com.example.app"}'
jidr logcat Phone -n 200 --filter ActivityManager:I
jidr logcat Phone --clear
```

`media.screenshot` takes an optional `"path"`; without it the PNG goes to the user's Pictures/Jidr folder. Taps, swipes, text and the UI tree go through `adb -s <serial> shell input …` or the MCP tools; see jidr-app-testing.

Other protocol commands work the same way through `jidr call`: `network.set`, `gps.set`, `battery.set`, `clipboard.set`, `media.record` / `media.record_stop` and more. `jidr audio inject <device> <file.wav>` plays a WAV as the device's microphone (`jidr audio stop <device>` ends it). `jidr commands` lists them all. A call with a missing param fails with `missing field <name>`, which tells you what to add.

## Snapshots, stop and clean up

```bash
jidr snapshot save Phone before-login
jidr snapshot load Phone before-login
jidr snapshot list Phone
jidr device stop Phone                       # saves the quick-boot state
jidr device delete Phone --yes               # only a device you created
```

## MCP instead of the CLI

If the Jidr MCP server is connected (Claude Code: `claude mcp add jidr -- jidr mcp`), its tools do the same: `command_run` runs any protocol command (`device.create`, `device.start`…), and `screenshot`, `tap`, `swipe`, `type_text`, `press_key`, `ui_dump` and `logcat_tail` drive a device directly. `screenshot` returns the image itself, so you don't need to read a file. Devices created through `command_run device.create` are agent-owned automatically.

## Exit codes (with `--json`)

0 ok · 1 unexpected · 2 usage (bad params, missing `--yes`) · 3 not found or not running (start it) · 4 busy (another job holds the device) · 5 boot timeout · 6 SDK, download or license · 7 root failed · 8 stale daemon (rerun with `--restart-stale`) · 9 resource exhausted (the machine's RAM budget is full; see jidr-agent-devices).

## Don't

- Don't delete, reset or wipe a device you didn't create in this session (`device delete`, `device reset`, `app.uninstall`, `snapshot delete`) without asking. Devices you created are yours to clean up; say what you removed.
- Don't stop a device the user is using or watching. Leave the user's devices as you found them.
- Don't change the root mode of the user's devices: `jidr root set` cold-boots them.
- Don't create Jidr devices with Android Studio's tools or in its AVD folder. Jidr keeps its own SDK and devices and only reads Android Studio's.
