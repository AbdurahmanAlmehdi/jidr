# Contributing to Jidr

Thanks for helping. Bug reports, fixes, features and docs are all welcome. This page covers setting up, the rules the code follows, and how a change gets merged.

## Before you start

- **Bugs:** open an issue with your macOS version, Mac model, the Android image (for example `35:google_apis`), the command or steps, and what you saw. `jidr --json …` output and the daemon log help.
- **Features:** check the issues labelled [`upcoming feature`](https://github.com/AbdurahmanAlmehdi/jidr/labels/upcoming%20feature) first, then open an issue to agree on the approach before writing a large change.
- **Security issues:** don't open a public issue; follow [SECURITY.md](SECURITY.md).

## Setup

You need an Apple Silicon Mac with macOS 14 or later for the app and the arm64 emulator tests. The CLI, the daemon and the offline tests also build on Linux.

```bash
brew install protobuf                     # protoc, for the gRPC bindings
curl https://sh.rustup.rs -sSf | sh       # rustup installs the pinned toolchain on first build
cargo install cargo-nextest --locked      # the test runner
git clone https://github.com/AbdurahmanAlmehdi/jidr && cd jidr
cargo xtask ci                            # fmt, clippy, tests, licence check
```

For the emulator tests you need an Android SDK with the emulator and one system image. Android Studio's SDK works and is only read, never modified. The tests use `system-images;android-31;google_apis;arm64-v8a` by default; override it with `JIDR_TEST_SDK`, `JIDR_TEST_IMAGE` and `JIDR_TEST_API`.

```bash
cargo nextest run -p jidr-widgets --features emulator
```

For the site: `cd site && npm ci && npm run dev`.

## How the code is organised

[AGENTS.md](AGENTS.md) has the architecture, a map of every crate, the licensing split and the protocol rules. Product and architecture decisions are in [docs/decisions/](docs/decisions/). If your change contradicts one, raise it in an issue first.

## Rules the code follows

- **Never crash.** `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!` and `dbg!` are denied outside tests. Return `jidr_core::JidrError`.
- **No `unsafe`** unless a crate truly needs it, allowed locally with a comment explaining why.
- **Every bug fix comes with a regression test.**
- **Split licensing.** The GUI crates are GPL-3.0-only because they link Slint; everything else is Apache-2.0 and must never depend on Slint or the GUI. `cargo xtask license-check` enforces it.
- **Clean room.** Use only Genymotion's public docs, the public AOSP emulator protos and docs, and behaviour you observe by running the SDK tools. Never decompile or inspect Genymotion binaries, never copy GPL code (Magisk, rootAVD), never bundle Google's SDK, system images or Magisk. Details in [docs/clean-room.md](docs/clean-room.md).
- **Protocol changes** that touch the wire format, an existing command, error codes or events need a row in the changelog of [docs/control-protocol.md](docs/control-protocol.md). Regenerate the command reference with `JIDR_UPDATE_DOCS=1 cargo nextest run -p jidr-daemon protocol_doc`.
- **Android Studio's SDK and AVDs stay read-only.** Tests create AVDs in temporary directories.

## Pull requests

1. Fork, branch from `main`, and keep the change focused.
2. Write [conventional commits](https://www.conventionalcommits.org/): `feat(widgets): …`, `fix(root): …`, `docs: …`, `test: …`, `ci: …`. The type decides the next version and the release notes.
3. Run `cargo xtask ci` and, if you touched a crate with emulator tests and have an image, its emulator tests.
4. Update the docs your change affects.
5. Open the pull request against `main`. CI runs fmt, clippy, tests and the licence check on Linux, clippy on macOS, and the emulator tests on Linux KVM (API 30, 33 and 35). PRs that only touch docs, the site or Markdown skip the emulator jobs.
6. A maintainer review is required before merging.

Emulator tests on shared CI runners can be slow. If one fails in code you didn't touch, say so in the PR; a maintainer can re-run it.

## Releases

Releases are cut from `main` by a release pull request that collects the changes since the last version. Merging it tags the version and builds the signed, notarized DMG. Contributors don't need to do anything for this beyond writing conventional commits.

## Using an AI coding agent

You're welcome to. Point it at [AGENTS.md](AGENTS.md); `CLAUDE.md` does that for Claude Code. You remain responsible for the change: review it, run the checks, and make sure it follows the clean-room rules.

## Licence of contributions

By contributing you agree that your contribution is licensed under the licence of the files you change: Apache-2.0, or GPL-3.0-only for `crates/jidr-ui` and `apps/jidr-gui`.
