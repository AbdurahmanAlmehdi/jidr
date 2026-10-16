# Jidr (جذر): rules for AI coding agents

Jidr is an open-source Genymotion Desktop alternative in Rust with one-click rooted Android devices. These are the rules an agent (and any contributor) follows when changing it. [CONTRIBUTING.md](CONTRIBUTING.md) has the human-facing version with setup steps. Product and architecture decisions are in [docs/decisions/log.md](docs/decisions/log.md) (cited as `D<n>`) and the ADRs next to it. Don't change a decision as a side effect of other work: propose it in an issue first.

## Architecture in one paragraph

`jidr daemon` owns all device state and drives the AOSP emulator (gRPC `EmulatorController`, the telnet console, `adb`). The GUI (`apps/jidr-gui`), the CLI (`apps/jidr`) and the MCP server (`jidr mcp`) are thin clients over JSON-RPC 2.0 / NDJSON on a Unix socket. Every user action is a protocol command; UI-only view state (window size, zoom, panel layout) stays in the GUI. Providers (`jidr-daemon` core, `jidr-widgets`, `jidr-root`) register commands through the `CommandProvider` trait in `jidr-core`.

## Repository map

| Path | Contents | Licence |
|---|---|---|
| `crates/jidr-core` | model, error codes, protocol types, the `CommandProvider` trait | Apache-2.0 |
| `crates/jidr-proto` | gRPC code from the emulator's `emulator_controller.proto` / `snapshot_service.proto` | Apache-2.0 |
| `crates/jidr-adb` | wraps the `adb` binary | Apache-2.0 |
| `crates/jidr-sdk` | SDK discovery and install, license consent, AVDs, hardware profiles, download cache | Apache-2.0 |
| `crates/jidr-backend` | `Backend` trait and the AOSP emulator backend (launch, ports, boot wait, console, gRPC) | Apache-2.0 |
| `crates/jidr-daemon` | command registry, JSON-RPC server, jobs, events, locks | Apache-2.0 |
| `crates/jidr-client` | protocol client used by the CLI, the GUI and MCP | Apache-2.0 |
| `crates/jidr-mcp` | MCP server | Apache-2.0 |
| `crates/jidr-widgets` | provider for sensors, phone, network, GPS, dev tools, media, snapshots | Apache-2.0 |
| `crates/jidr-cpio` | ramdisk cpio and gz/lz4/xz codecs | Apache-2.0 |
| `crates/jidr-root` | provider for root modes (adb, Magisk, verify, revert, build props) | Apache-2.0 |
| `crates/jidr-ui` | Slint GUI library | **GPL-3.0-only** |
| `apps/jidr` | the `jidr` binary: CLI, `daemon` and `mcp` subcommands | Apache-2.0 |
| `apps/jidr-gui` | the GUI binary | **GPL-3.0-only** |
| `xtask` | `ci`, `license-check`, `e2e-mac`, `release` | Apache-2.0 |
| `site/` | the jidr.ly site (Astro, not in the Cargo workspace) | Apache-2.0 |
| `docs/design/` | design tokens (`tokens.json`, the single source for app and site), CSS and component specs | — |

Dependency versions live in the root `[workspace.dependencies]`; crates use `foo.workspace = true`.

## Licensing (D18)

- Apache-2.0 everywhere except `crates/jidr-ui` and `apps/jidr-gui`, which are GPL-3.0-only because they link Slint.
- The CLI, the daemon, MCP and every other Apache crate must never depend on Slint or the GUI crates. `cargo xtask license-check` (part of `cargo xtask ci`) enforces this.

## Clean-room rules (non-negotiable)

See [docs/clean-room.md](docs/clean-room.md). In short: use only Genymotion's public docs and the public AOSP emulator protos and docs, plus behaviour you observe by running the SDK tools. Never decompile, string-dump or inspect Genymotion binaries. Never copy GPL code (Magisk, rootAVD); you may read their scripts to learn the documented ramdisk layout. Never bundle Google SDK binaries, system images or Magisk. No Google, Android or Genymotion logos. Log any non-obvious source in the clean-room file.

## Code rules

- Rust edition 2024, toolchain pinned in `rust-toolchain.toml`.
- Never crash: `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` and `dbg!` are denied outside tests (workspace lints and `clippy.toml`). Return typed errors (`jidr_core::JidrError`). Integration-test files (`tests/*.rs`) start with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`, because clippy's test allowance only covers `#[test]` fns and `#[cfg(test)]` modules.
- `unsafe` is denied by default. A crate that truly needs it allows it locally with a comment explaining why.
- Every bug fix gets a regression test.
- Emulators launch with `-gpu host` on macOS (ADR-0001: `auto` falls back to software rendering under `-no-window`); CI on Linux KVM uses `swiftshader_indirect`.
- Android Studio's SDK and AVDs are read-only (D3). Jidr's own SDK lives in `~/Library/Application Support/Jidr/sdk`. Tests create AVDs in temp dirs, never in `~/.android/avd`.
- Never accept a Google SDK license on a user's behalf outside a test context.

## Tests

- `cargo nextest run -p <crate>` runs unit and integration tests offline. `jidr_daemon::testing::TestDaemon` gives a daemon with a fake backend and a fake SDK.
- Tests that need a running emulator sit behind each crate's `emulator` feature (`cargo nextest run -p <crate> --features emulator`). They read:
  - `JIDR_TEST_SDK`: the SDK root (default `~/Library/Android/sdk`, used read-only);
  - `JIDR_TEST_IMAGE`: the sdkmanager package of the image (default on Apple Silicon `system-images;android-31;google_apis;arm64-v8a`);
  - `JIDR_TEST_API`: its API level.
- On a pull request, CI runs the emulator tests on Linux KVM with x86_64 images (API 30, 33 and 35). `cargo xtask e2e-mac` is the arm64 release gate on an Apple Silicon Mac (D28).

## Protocol

The control protocol is frozen at the tag `protocol-v0.1` ([docs/control-protocol.md](docs/control-protocol.md)). Any change to the wire format, an existing command's params or result, error codes or event topics needs a row in that file's changelog. New commands from providers are additive. The generated command reference is kept in sync by `cargo nextest run -p jidr-daemon protocol_doc` (`JIDR_UPDATE_DOCS=1` rewrites it).

## Before you open a pull request

1. `cargo xtask ci` passes: fmt, clippy, tests and the licence check.
2. If you touched a crate with emulator tests, run them for that crate if you have an image.
3. Docs that describe the change are updated (`docs/cli.md`, `docs/mcp.md`, `docs/root.md`, the protocol changelog).
4. One conventional commit per logical change (`feat(daemon): …`, `fix(widgets): …`, `docs: …`). Release notes are generated from them.
5. Never push to `main`; open a pull request. Don't push or publish anything without the maintainer asking.

## Useful commands

```bash
cargo xtask ci                                   # fmt, clippy, tests, license-check
cargo xtask license-check                        # D18 split-licensing guard
cargo nextest run -p <crate>                     # unit + integration tests
cargo nextest run -p <crate> --features emulator # needs an emulator image
cargo xtask e2e-mac                              # release gate on Apple Silicon (D28)
cd site && npm ci && npm run build               # the site
```
