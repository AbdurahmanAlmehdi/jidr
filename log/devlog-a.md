# Stream A devlog (Core platform)

Branch `stream/a-core`, worktree `.claude/worktrees/a-core`, `CARGO_TARGET_DIR=target/agent-a`.

Environment notes (2026-10-13):
- Rust via Homebrew `rustup` (keg-only): put `/opt/homebrew/opt/rustup/bin` on `PATH`. Toolchain pinned in `rust-toolchain.toml`.
- `protoc` 36.2 and `cargo-nextest` 0.9.148 from Homebrew.
- Local arm64 images (Studio SDK, read-only): API 31 `google_apis`, API 34 `default`, API 36 `google_apis`, API 36 `google_apis_playstore_ps16k`. API 35 is x86_64 only. Per Abdurahman: use the installed arm64 images (31 + 36) for local emulator tests for now; 30/33/35 come later.

---

## Phase 0: Workspace bootstrap — plan

1. Root `Cargo.toml`: workspace with all 13 crates + `apps/jidr`, `apps/jidr-gui`, `xtask`; shared `[workspace.package]`, `[workspace.dependencies]` (versions for every stream, so B/C only write `x.workspace = true`), `[workspace.lints]` denying `unwrap_used`/`expect_used`/`panic`; `clippy.toml` allows them in tests.
2. Stubs: `src/lib.rs` per crate, `src/main.rs` for apps, no heavy deps yet (keeps Phase 0 fast). GPL crates get `license = "GPL-3.0-only"` and `LICENSE-GPL`.
3. `xtask`: `ci` (fmt --check, clippy -D warnings, nextest, license-check), `license-check` (walks `cargo metadata`: no Apache crate may reach `slint`/`i-slint-*`/`jidr-ui`; license fields match D18), `e2e-mac` stub. License-check logic is a pure function tested against synthetic metadata, including "slint added to apps/jidr" → fails.
4. `.cargo/config.toml` alias `xtask`; `rust-toolchain.toml`.
5. `.github/workflows/ci.yml`: fmt/clippy/test on macOS + Linux; Linux KVM job running `--features emulator` with an x86_64 image.
6. Docs: `AGENTS.md` (+ `CLAUDE.md` pointing to it), `docs/clean-room.md`, `docs/gaps.md`, `docs/parity.md`, `docs/decisions/README.md`; `LICENSE-APACHE`, `NOTICE`.
7. Validate with `cargo xtask ci`, set Status: Done, commit.

## Phase 0: Workspace bootstrap — done (2026-10-13)

Built:
- 15-member workspace (13 crates from the plan's table + `apps/jidr-gui` + `xtask`), each stubbed, each with an `emulator` feature. `[workspace.dependencies]` pins shared versions (tokio 1.53, tonic/prost 0.14, schemars 1.2, rmcp 3.5, slint 1.18, clap 4.6 …) so B/C never edit the root manifest.
- Never-crash lints in `[workspace.lints.clippy]` (`unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro` = deny), allowed in tests via `clippy.toml`; `unsafe_code` = deny.
- `cargo xtask ci` (fmt, clippy -D warnings with all features, nextest, license-check), `cargo xtask license-check` (walks `cargo metadata`: no Apache crate may reach `slint`/`i-slint-*`/`jidr-ui`/`jidr-gui` via normal/build deps; license fields match D18), `cargo xtask e2e-mac` (emulator tests + e2e scripts), `xtask/src/release.rs` stub for Stream C.
- Tests: 6 synthetic-metadata cases (incl. "slint added to apps/jidr" → fails, transitive `i-slint-core` via a build dep → fails, dev-dep allowed) + the real workspace passes.
- `.github/workflows/ci.yml`: macOS + Linux check job; Linux KVM emulator job (API 30/33/35 x86_64 `google_apis`) that sets `JIDR_TEST_SDK/IMAGE/API`.
- `AGENTS.md` (+ `CLAUDE.md` → `@AGENTS.md`), `docs/{clean-room,gaps,parity}.md`, `docs/decisions/README.md`, `LICENSE-APACHE`, `crates/jidr-ui/LICENSE-GPL`, `apps/jidr-gui/LICENSE-GPL`, `NOTICE`.

Skipped / notes:
- CI workflow is unvalidated until the repo is pushed (not pushing without asking).
- Emulator-test env contract (`JIDR_TEST_SDK`, `JIDR_TEST_IMAGE`, `JIDR_TEST_API`) is defined in AGENTS.md; local default is the Studio SDK read-only + API 36 `google_apis` arm64.
