# Jidr (جذر): agent and contributor rules

Jidr is an open-source Genymotion Desktop alternative in pure Rust with one-click rooted Android devices. The source of truth is [docs/plan/2026-10-13-feat-jidr-emulator-suite-plan.md](docs/plan/2026-10-13-feat-jidr-emulator-suite-plan.md): its Decisions log (D1–D34), crate table, protocol section and Implementation Phases. Don't change a decision without asking Abdurahman.

## Architecture in one paragraph

`jidr daemon` owns all device state and drives the AOSP emulator (gRPC `EmulatorController`, telnet console, `adb`). The GUI (`apps/jidr-gui`), the CLI (`apps/jidr`) and the MCP server (`jidr mcp`) are thin clients over JSON-RPC 2.0 / NDJSON on a Unix socket. Every user action is a protocol command; UI-only view state (window size, zoom, panel layout) stays in the GUI. Providers (`jidr-daemon` core, `jidr-widgets`, `jidr-root`) register commands through the `CommandProvider` trait in `jidr-core`.

## Stream ownership (D27)

Each stream works in its own git worktree with its own `CARGO_TARGET_DIR=target/agent-<stream>` and edits only its own files. Need something from another stream? Add a row to [docs/gaps.md](docs/gaps.md) and work on something else.

| Stream | Owns |
|---|---|
| A: Core platform | root `Cargo.toml`, `Cargo.lock`, `.cargo/`, `rust-toolchain.toml`, `clippy.toml`, `xtask/` (except `xtask/src/release.rs`), `crates/jidr-{core,proto,adb,sdk,backend,daemon,client,mcp}`, `apps/jidr`, `.github/workflows/ci.yml`, `spikes/display`, `assets/profiles/`, `scripts/e2e-cli.sh`, `scripts/e2e-studio-untouched.sh` |
| B: Player & widgets | `crates/jidr-ui` (except `src/root/**`), `apps/jidr-gui`, `crates/jidr-widgets` |
| C: Root, ship & site | `spikes/playstore-root`, `crates/jidr-cpio`, `crates/jidr-root`, `crates/jidr-ui/src/root/**`, `assets/magisk-pins.toml`, `docs/root.md`, `scripts/e2e-root.sh`, `site/`, `packaging/`, `xtask/src/release.rs`, `.github/workflows/release.yml` |

Shared docs (`docs/gaps.md`, `docs/parity.md`, `docs/clean-room.md`, `docs/decisions/`) may be appended to by any stream. Each stream keeps its own `log/devlog-<stream>.md`.

Dependencies: versions live in the root `[workspace.dependencies]`. Use `foo.workspace = true`. A stream that needs a crate that isn't listed there adds it with an explicit version in its own crate manifest, and files a gap if it should be shared.

## Licensing (D18)

- Apache-2.0 everywhere except `crates/jidr-ui` and `apps/jidr-gui`, which are GPL-3.0-only because they link Slint.
- The CLI, daemon, MCP and every other Apache crate must never depend on Slint or the GUI crates. `cargo xtask license-check` (part of `cargo xtask ci`) enforces this.
- `jidr-widgets` is Apache-2.0 and must not depend on Slint.

## Clean-room rules (non-negotiable)

See [docs/clean-room.md](docs/clean-room.md). In short: use only Genymotion's public docs and the public AOSP emulator protos and docs. Never decompile, string-dump or inspect Genymotion binaries. Never copy GPL code (Magisk, rootAVD). You may read their scripts to learn the documented ramdisk layout. Never bundle Google SDK binaries, system images or Magisk. No Google, Android or Genymotion logos.

## Code rules

- Rust edition 2024, toolchain pinned in `rust-toolchain.toml`.
- Never crash: `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` and `dbg!` are denied outside tests (workspace lints + `clippy.toml`). Return typed errors (`jidr_core::JidrError`). Integration-test files (`tests/*.rs`) start with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`, because clippy's test allowance only covers `#[test]` fns and `#[cfg(test)]` modules.
- `unsafe` is denied by default. A crate that truly needs it allows it locally with a comment explaining why.
- Every bug fix gets a regression test.
- Emulators launch with `-gpu host` on macOS (ADR-0001: `auto` falls back to software rendering under `-no-window`); CI on Linux KVM uses `swiftshader_indirect`.
- Tests that need a running emulator go behind the crate's `emulator` feature. They read:
  - `JIDR_TEST_SDK`: the SDK root (default: Android Studio's SDK at `~/Library/Android/sdk`, used **read-only**)
  - `JIDR_TEST_IMAGE`: the sdkmanager package of the image (default on Apple Silicon: `system-images;android-31;google_apis;arm64-v8a`, the only complete arm64 `google_apis` image on the dev Mac today)
  - `JIDR_TEST_API`: its API level
- The Android Studio SDK and AVDs are read-only (D3). Tests create AVDs in temp dirs, never in `~/.android/avd`.
- The private SDK lives in `~/Library/Application Support/Jidr/sdk`. Never accept a Google SDK license on the user's behalf outside a test context.

## Finish checklist (every phase)

1. Plan the phase in `log/devlog-<stream>.md` before writing code.
2. Run the phase's **Validation** command; it must exit 0 (or the manual steps are written up in the devlog with numbers).
3. Run `cargo xtask ci`. It must pass.
4. Set the phase's **Status:** to `Done` in the plan.
5. Commit locally, one conventional commit per phase (`feat(daemon): …`). Never push without asking.
6. Append a devlog entry: what was built, what was skipped, open issues.

## Useful commands

```bash
cargo xtask ci                       # fmt, clippy, tests, license-check
cargo xtask license-check            # D18 split-licensing guard
cargo nextest run -p <crate>         # unit + integration tests
cargo nextest run -p <crate> --features emulator   # needs an emulator image
cargo xtask e2e-mac                  # release gate on Apple Silicon (D28)
```
