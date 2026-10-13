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

---

## Phase 1: Display + input spike — plan

Throwaway crate `spikes/display` (`spike-display`, GPL-3.0-only because it links Slint; added to license-check's GPL list, never shipped).

1. `build.rs` generates a tonic client from the SDK's `emulator_controller.proto` (no `jidr-proto` yet, that's 3a).
2. AVD: `--avd <name>` uses an existing AVD; `--create` writes a spike-only AVD (API 36 `google_apis` arm64, 1080×2400, 420 dpi) under `target/agent-a/spike-avd` via `ANDROID_AVD_HOME`, so neither the Studio SDK nor `~/.android/avd` is written.
3. Launch `emulator -no-window -grpc <port> -grpc-use-token -no-boot-anim -no-snapshot`, read the token from the discovery file `~/Library/Caches/TemporaryItems/avd/running/pid_<pid>.ini`, wait for `getStatus.booted`.
4. Display: `streamScreenshot(RGBA8888, width = window px)` → newest-frame slot → `invoke_from_event_loop` → `SharedPixelBuffer<Rgba8Pixel>` → Slint `Image`. Count frames shown per second.
5. Input: Slint `TouchArea` → map to device px → `streamInputEvent` (touch down/move/up).
6. Measurements, automated so the numbers are reproducible:
   - `--bench-fps`: open Settings, inject continuous swipes over gRPC for 10 s, report frames received / displayed per second and process CPU (self + emulator via `ps`).
   - `--bench-latency`: enable `show_touches`, tap a flat area N times, time from send → first displayed frame whose probe pixel changed; report median / p90.
   - Run at 400 px (1× frames) and 800 px (Retina 2×) widths; fall back to MMAP transport if the budget is missed.
7. Record numbers + decision in `docs/decisions/0001-frame-transport.md`.

## Phase 1: Display + input spike — done (2026-10-13), **pass**

Built: `spikes/display` (GPL, throwaway) with `--create` (spike-only AVD under `target/agent-a/spike-avd`), `--bench fps`, `--bench latency`, `--gpu`, `--frame-width`, `--dump`. Numbers and decision in `docs/decisions/0001-frame-transport.md`.

Results (M1 Pro, API 31 google_apis arm64 at 1080×2400): with `-gpu host`, 59.9 fps active scrolling at both 400 px and 800 px frames, touch → changed frame on the UI thread 13.5 ms median (p90 16–26 ms), estimated input-to-photon ≈ 35 ms. Inline RGBA + `SharedPixelBuffer` is enough; no MMAP or wgpu needed.

Findings that bind later phases:
- `-gpu host` must be the macOS default (Phase 3b). `auto` under `-no-window` = software GL: ~40 fps, 2× emulator CPU.
- `ImageFormat` needs width **and** height; width alone returns full-res frames.
- Frames are top-down despite the proto comment.
- The local `system-images/android-36/google_apis/arm64-v8a` (and API 34 `default`, Baklava) directories are empty. API 31 `google_apis` arm64 is the local emulator-test image until 30/33/35 are installed.

Skipped / open:
- Manual "click Settings in the window" check: computer use can't drive a bare (non-.app) binary. Needs Abdurahman: `CARGO_TARGET_DIR=target/agent-a cargo run --release -p spike-display -- --create --gpu host`, then click Settings.
