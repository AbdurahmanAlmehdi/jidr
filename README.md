# Jidr (جذر)

Android on your Mac, rooted in a minute.

Jidr creates and runs virtual Android phones and tablets on Apple Silicon Macs. Pick **Full root** and Magisk is patched in and verified for you; pick **Shell root** for `adb root`; or keep the device stock. It's a free, open-source alternative to Genymotion Desktop, written in Rust, with a desktop app, a CLI and an MCP server for AI agents.

**Status:** pre-release. The first release (v0.1.0) is in preparation; until then, build from source. Site: [jidr.ly](https://jidr.ly).

## Features

- **Devices:** create, clone, start, stop and delete virtual devices from Google's emulator images (API 29–36, arm64). Jidr keeps its own SDK and never modifies Android Studio's SDK or AVDs.
- **One-click root:** None, Shell root (`adb root`) or Full root (Magisk, patched into a per-device copy of the ramdisk and verified with `su -c id`). Revert at any time. Play Store images get a guided beta.
- **Player window:** the device screen with mouse and keyboard input, drag-and-drop for APKs and files, and logcat.
- **Widgets:** battery, rotation and motion sensors, network (Wi-Fi, mobile data, speed and delay profiles), calls, SMS and signal bars, GPS with GPX/KML route replay on an OpenStreetMap map, fingerprint, the host webcam as the camera, clipboard and developer toggles.
- **Media and state:** PNG screenshots, MP4 recordings, and snapshots tagged with the root mode they were taken in.
- **Build identity:** set build properties (model, brand, fingerprint…) on Full-root devices.
- **CLI:** every action is a `jidr` command, with `--json` output and stable exit codes for CI.
- **MCP server:** `jidr mcp` lets Claude and other MCP clients see and drive a device: screenshot, tap, type, UI tree, logs.

No accounts, no telemetry, no paid tier.

## Requirements

- macOS 14 or later on Apple Silicon.
- About 10 GB of free disk space for the SDK and one system image.

## Build from source

```bash
git clone https://github.com/AbdurahmanAlmehdi/jidr
cd jidr
cargo build --release -p jidr -p jidr-gui
```

You need `rustup` (it installs the toolchain pinned in `rust-toolchain.toml`) and `protoc` (`brew install protobuf`). [CONTRIBUTING.md](CONTRIBUTING.md) has the full setup and how to run the tests.

## Quick start (CLI)

```bash
jidr image install 35 --accept-licenses   # download an API 35 image; you accept Google's license
jidr device create Phone --image 35       # create a device
jidr device start Phone                   # boot it and wait until it's ready
jidr root set Phone magisk                # Full root with Magisk, verified
```

Add Jidr to Claude Code as an MCP server:

```bash
claude mcp add jidr -- jidr mcp
```

## Documentation

- [CLI reference](docs/cli.md)
- [MCP tools](docs/mcp.md)
- [Root modes](docs/root.md)
- [Control protocol](docs/control-protocol.md): the JSON-RPC protocol the app, the CLI and the MCP server share
- [Design system](docs/design/README.md)
- [Decisions](docs/decisions/log.md) and [architecture decision records](docs/decisions/)
- [Feature parity with Genymotion Desktop](docs/parity.md)
- [Clean-room policy](docs/clean-room.md)

## Licensing

Jidr is split-licensed. The desktop app (`crates/jidr-ui`, `apps/jidr-gui`) is **GPL-3.0-only** because it links Slint. Everything else (CLI, daemon, MCP server, libraries, site) is **Apache-2.0**. See `LICENSE-APACHE`, the `LICENSE-GPL` files in the two GUI crates, and `NOTICE`.

Jidr downloads the Android Emulator and system images from Google under the Android SDK License, which you accept in the app. It downloads Magisk (GPL-3.0) only when you choose Full root. None of these are bundled.

## Contributing

Contributions are welcome: read [CONTRIBUTING.md](CONTRIBUTING.md). Report security issues as described in [SECURITY.md](SECURITY.md). If you work with an AI coding agent, point it at [AGENTS.md](AGENTS.md).

## Trademarks

Android is a trademark of Google LLC. Genymotion is a trademark of its owner. Jidr is not affiliated with Google or Genymotion.
