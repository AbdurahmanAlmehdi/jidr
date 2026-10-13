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

---

## Phase 2: Core model + SDK + AVD — plan

`jidr-core`:
- `model.rs`: Device, DeviceStatus, HardwareProfile, SystemImage, ImageRef, RootMode/RootState/RootHealth, Snapshot, Job/JobState. All `serde` + `schemars`.
- `error.rs`: `ErrorCode` (ok 0, unexpected 1, usage 2, not_found 3, busy 4, boot_timeout 5, sdk 6, root_failed 7, daemon_stale 8 = CLI exit codes) + `JidrError { code, message, data }`.
- `paths.rs`: `JidrPaths` (data / sdk / avd / cache / logs / consent / socket), `JIDR_HOME` override for tests.

`jidr-sdk` (deviation from the plan's "sdkmanager wrapper", recorded here): install natively. Parse Google's repository XMLs (`repository2-3.xml`, `sys-img/<tag>/sys-img2-3.xml`), download archives with progress, verify SHA-1, unzip into the private SDK. Why: sdkmanager needs a Java runtime that a Mac with no Android Studio doesn't have, and its license-hash format (`licenses/<id>`) isn't reproducible from the published XML text (tested: SHA-1 of the raw, trimmed, collapsed and CRLF text never matches the hash sdkmanager writes). Jidr's own consent store (D8) keys consent by SHA-256 of the exact license text shown, re-prompting when it changes.
- `host.rs`: host ABI (arm64-v8a / x86_64), hypervisor check (`kern.hv_support` / `/dev/kvm`), free disk space.
- `sdk.rs`: `Sdk { root, kind: Private | StudioReadOnly }`, detection, tool paths; Studio SDK never written.
- `repo.rs`: XML → `RemotePackage` (path, revision, archives per host, license ref, api/tag/abi) + licenses.
- `catalog.rs`: image catalog: API 29–36 supported, 28 best-effort (D23), host-ABI filter, installed state.
- `license.rs`: `ConsentStore` (JSON, per license id + sha256), `pending_licenses(packages)`.
- `install.rs`: download (https or file:// for tests) → sha1 check → unzip into a staging dir → atomic rename; disk-space precheck; progress callback; cancel flag.
- `avd.rs`: `ConfigIni` (order- and format-preserving round-trip), create from profile + image, edit, clone (copy minus locks/runtime/snapshots; overlays use relative backing files), delete, factory reset, list. AVDs live in Jidr's own AVD home, never `~/.android/avd`.
- `profiles.rs` + `assets/profiles/*.toml`: built-in generic profiles (D24). SDK device-definition import is deferred (definitions live inside Studio jars / `~/.android/devices.xml`).
- Tests: config.ini round-trip, consent store, ABI filter, XML parsing (synthetic fixture), installer against a `file://` fixture zip. Emulator feature: create → clone → delete against `JIDR_TEST_SDK`, boot the clone to prove cloning works.
- `scripts/e2e-studio-untouched.sh`: fingerprint (path, size, mtime) of the Studio SDK + `~/.android/avd` before/after running a command.

## Phase 2: Core model + SDK + AVD — done (2026-10-13)

Built:
- `jidr-core`: `ErrorCode` (exit codes 1–8, JSON-RPC codes: usage → -32602, others -3200N), `JidrError`/`JidrResult`/`ResultExt::ctx`, the model (Device, DeviceStatus, DeviceRuntime, HardwareProfile, ImageRef, SystemImage, Support (D23), RootMode/RootState/RootHealth, Snapshot, Job/JobState), `JidrPaths` (`JIDR_HOME` override).
- `jidr-sdk`: `host` (ABI, HVF/KVM, free space), `sdk` (private vs Studio read-only, complete-image detection, `SdkSet::tools/find_image`), `repo` (repository XML parser), `catalog` (ABI/API/channel filter, offline installed images), `license` (consent store by SHA-256 of the exact text, `accept_hash` for headless), `install` (native installer: https/file:// download with progress + cancel, SHA-1 check, single-root unzip, atomic move, staging cleanup, never into Studio's SDK), `avd` (byte-identical `config.ini` round-trip, create/edit/clone/delete/factory reset/list/disk usage, running-AVD protection via `hardware-qemu.ini.lock` pid), `profiles` (4 generic built-ins in `assets/profiles/`).
- `examples/catalog.rs`: the live Google listing parses (566 packages; arm64 images for API 28–36 incl. 30/33/35).
- `scripts/e2e-studio-untouched.sh`: (path, size, mtime) fingerprint of the Studio SDK + `~/.android/avd` around a command.

Validation: `cargo nextest run -p jidr-core -p jidr-sdk` (32 passed) && `--features emulator` (create → clone → **clone boots in 24 s** → delete) && `scripts/e2e-studio-untouched.sh` (296,062 Studio files unchanged).

Deviation (recorded in the plan section above): no sdkmanager wrapper. sdkmanager needs Java, and its `licenses/<id>` hash is not reproducible from the published XML text (raw/trimmed/collapsed/CRLF SHA-1 all differ from what sdkmanager writes), so Jidr's consent is its own (D8 intent kept: real text shown, consent per hash, re-prompt on change).

Skipped / open:
- SDK device-definition import (D24 "imported as-is"): the definitions live inside Studio's jars; deferred.
- 16 KB page-size images (`*_ps16k`) appear in the catalog; untested (research summary). Consider hiding them until tested.
- Found: Studio's `system-images/android-36/google_apis_playstore_ps16k` has `ramdisk.img.backup`, i.e. rootAVD patched that shared image in place. Relevant for Stream C's pristine-hash logic.

---

## Phase 3a: gRPC bindings + adb — plan

- `jidr-proto`: `build.rs` compiles `emulator_controller.proto` + `snapshot_service.proto` (+ `snapshot.proto`) from the installed SDK (`ANDROID_SDK_ROOT`/`ANDROID_HOME`/default), else the vendored copies in `proto/` (or when `JIDR_PROTO_VENDORED=1`). Client + server code (server for fake emulators in tests). `BearerToken` interceptor, `clients()` with a 64 MB decode limit. Tests: the vendored copies compile on their own (tempdir out_dir), the protos declare every RPC we use.
- `jidr-adb`: `Adb` (+ `.device(serial)`, timeout), `raw/run`, devices, shell/shell_status, getprop, install (with `InstallFailure` / signature-mismatch data), uninstall, push/pull, logcat dump/clear/follow, root/unroot (production builds → `root_failed`), connect/disconnect, wait_for_device. Errors classified to not_found / busy / sdk / boot_timeout. Tests against a fake `adb` bash script.

## Phase 3a — done (2026-10-13)

Validation `cargo nextest run -p jidr-proto -p jidr-adb`: 15 passed; also passes with `JIDR_PROTO_VENDORED=1`.
Notes: the generated code addresses `emulator_snapshot` as `super::super::super::…`, so modules mirror the proto packages (`android::emulation::control`, re-exported as `control`).

---

## Phase 3b: Backend launch — plan

Gaps check: Stream C (branch `claude/trusting-grothendieck-401ea9`) filed C1, FYI only: `jidr-cpio` deps change `Cargo.lock`; regenerate on merge. Nothing blocking.

- `jidr-core`: move the cancel flag type here (`Cancel = Arc<AtomicBool>`), shared by SDK installs, boots and jobs.
- `jidr-backend`:
  - `Backend` trait (async-trait, object-safe so the daemon can use a fake): `start(spec, opts, progress, cancel) -> InstanceInfo`, `stop(info)`, `is_alive(info)`.
  - `LaunchSpec { avd, avd_home, sdk_root, emulator, adb, log_dir }`, `LaunchOptions { extra_args, ramdisk, cold_boot, wipe, gpu, audio, snapshot_save, boot_timeout }` (`ramdisk` is Stream C's Magisk hook).
  - `ports`: allocator with an in-process reservation set; console/adb pairs 5554–5584 first (adb auto-discovers them), then higher pairs with `adb connect 127.0.0.1:<adb>` (D9); gRPC port from the OS.
  - `discovery`: find the emulator's `pid_<pid>.ini` (macOS caches dir; Linux `$XDG_RUNTIME_DIR`/tmp candidates), match by pid or `port.serial`, read `grpc.token`.
  - `aosp`: spawn `-no-window -grpc <p> -grpc-use-token -no-boot-anim -port <c> -gpu host|swiftshader_indirect` (+ `-no-snapshot-load` for cold boot, `-wipe-data`, `-ramdisk`, `-no-snapshot-save`, `-no-audio`), env `ANDROID_AVD_HOME`/`ANDROID_SDK_ROOT`, stdout/stderr to a per-boot log; boot stages Spawned → GrpcReady → AdbOnline → Booted (`getStatus.booted` + `sys.boot_completed`); early exit / timeout / cancel kill the process and report the log path.
  - Stop: gRPC `setVmState(SHUTDOWN)` (same as closing the window: saves the quick-boot snapshot unless launched with `-no-snapshot-save`, D29), wait, then SIGKILL fallback.
  - `grpc::connect(info)`: authenticated `Controller`/`Snapshots` clients.
- Emulator test: boot API 31 (local image), read battery over gRPC, stop, then cold boot with `-ramdisk` = a copy of the pristine ramdisk and check the emulator's `hardware-qemu.ini` points at it; quick-boot snapshot exists after the first stop.

## Phase 3b: Backend launch — done (2026-10-13)

Built: `jidr-core::Cancel` (shared cancel flag; jidr-sdk switched to it); `jidr-backend`: object-safe `Backend` trait, `LaunchSpec`, `LaunchOptions` (ramdisk/cold_boot/wipe/gpu/audio/snapshot_save/extra_args/boot_timeout), `BootStage`, `InstanceInfo` (+ `runtime()`), `PortAllocator` (reservations, TCP serial past 5584), `discovery` (scan/parse/match), `grpc::connect`, `AospBackend` (spawn with a per-boot log, exit watcher per pid, staged boot wait, failure/timeout/cancel kill + port release, graceful stop via `setVmState(SHUTDOWN)` with SIGKILL fallback).

Validation `cargo nextest run -p jidr-backend --features emulator`: 11 passed. Emulator test (API 31 arm64): first boot 24 s → battery over gRPC → stop saves `snapshots/default_boot` → cold boot with a copied ramdisk via `LaunchOptions.ramdisk` (verified `disk.ramdisk.path` in the emulator's `hardware-qemu.ini`) → quick boot 13.5 s.

Bugs found by the emulator test (each has a regression test):
1. Discovery matched by console port alone, so a stale `pid_*.ini` (Studio's leftover, or the previous boot) supplied a wrong gRPC token and boot never completed → match pid, or console + our gRPC port; an Unauthenticated `getStatus` now fails immediately (`discovery::tests::stale_file_with_same_console_is_ignored`).
2. Emulator lock files are NUL-padded after the pid; `jidr-sdk` treated them as unparseable = held, so every stopped AVD looked busy (`avd::tests::nul_padded_lock_files`).

Open:
- Quick boot is 13.5 s vs the ≤ 5 s metric: gRPC is up at 1.0 s, adb online at 7.3 s (adb server's emulator scan interval), `Booted` at 13.5 s (`getStatus.booted` lags after a snapshot restore). Tune later (e.g. trust `sys.boot_completed` alone after a snapshot load, nudge adb discovery).
- Stop without saving the snapshot is only a launch-time choice (`snapshot_save=false`); a per-stop "don't save" needs another mechanism.

---

## Phase 3c: Backend resilience — plan

Gaps: only C1 (FYI). Nothing to serve.

- `console.rs`: telnet console client (`127.0.0.1:<console>`), banner + `auth <token>` from `~/.emulator_console_auth_token` (read-only; the emulator owns it), `OK` / `KO: …` replies. Typed helpers: `network speed|delay`, `gsm signal|data|voice`, `power ac|health|capacity|status`, `avd name`, `kill`. Unit-tested against an in-process fake console.
- Double launch: `start` refuses an AVD whose `hardware-qemu.ini.lock` names a live pid (Android Studio or another Jidr) and an AVD this backend is already launching → `busy`. `jidr-backend` takes `jidr-sdk` as a dependency for the lock check.
- `adopt.rs`: after a daemon restart, scan discovery files for live emulators whose `avd.dir` is under Jidr's AVD home, rebuild `InstanceInfo` (serial from the console port), claim their ports.
- `crash.rs`: `wait_for_crash(info, log_dir)` resolves on process exit (exit watcher for owned instances, `kill -0` polling for adopted ones) or 3 failed gRPC heartbeats in a row; writes `crash-<ts>.log` (reason + emulator log) and returns a `CrashReport`. No auto-restart (D9). The daemon aborts the watcher before an intentional stop.
- Emulator test (one boot): double launch refused → console `avd name`/`network speed`/`power ac` → a fresh backend re-adopts the instance (same pid/ports/token) → `kill -9` → crash reported with a saved log path.

## Phase 3c: Backend resilience — done (2026-10-13)

Built: `console` (telnet client: banner + `auth` from `~/.emulator_console_auth_token`, OK/KO parsing, network speed/delay, gsm signal/data/voice, power ac/health, avd name, kill; argument validation against injection), double-launch refusal in `start` (live `hardware-qemu.ini.lock` → busy, plus an in-flight set; `jidr-backend` now depends on `jidr-sdk`), `adopt` (discovery files under Jidr's AVD home with a live pid → `InstanceInfo`, ports claimed; Studio's emulators ignored), `crash` (`wait_for_crash`: exit watcher or `kill -0` polling + 3 missed gRPC heartbeats → `CrashReport` with a saved `crash-<ts>.log`; no restart).

Validation `cargo nextest run -p jidr-backend --features emulator`: 17 passed, also with both emulator tests in parallel (`--test-threads 4`). Emulator test: double launch → busy; console commands on the real emulator; a fresh backend re-adopts the instance (same pid/ports/token/serial); `kill -9` → `Exited` report + crash log, seen by both the owning and the adopting backend.

Bug found (regression test `discovery::tests::actual_ports_come_from_the_discovery_file`): two emulators launched at once by different processes both asked for console 5554; the loser silently bound another port and we talked to the wrong emulator's console. The discovery file (matched by pid or our reserved gRPC port) is now the source of truth for ports.

---

## Phase 4a: Daemon core — plan

Gaps: only C1 (FYI).

Provider contract (`jidr-core::provider`), what B and C build on:
- `CommandSpec { id, label, params (JSON Schema), destructive, longRunning, locksDevice }` (camelCase on the wire).
- `#[async_trait] trait CommandProvider { fn commands(); async fn call(&CallCtx, method, params) -> JidrResult<Value>; async fn on_booted(&CallCtx, &Device) {} }`. `on_booted` lets the root engine run `adb root` after every boot.
- `CallCtx { daemon: Arc<dyn DaemonContext>, progress, cancel }`; `DaemonContext` exposes paths, adb path, device lookup by name/id, `connect_info`, `update_device`, `start_device(cold_boot)`, `stop_device`, and (4b) events.
- `Device.launch: LaunchPrefs { ramdisk, extra_args, gpu, force_cold_boot }` is persisted. The root engine sets `ramdisk` + `force_cold_boot` and restarts (D29); the daemon applies them on every start.
- `parse_params::<T>()`: serde with `deny_unknown_fields` → `usage` (-32602) naming the key.
- B and C each export `pub fn provider() -> Arc<dyn CommandProvider>` from `jidr-widgets` / `jidr-root`; A wires them into `jidr daemon` when they land (gap rows A1/A2 filed for B and C).

`jidr-daemon`:
- `rpc`: JSON-RPC 2.0 types; -32700 parse, -32600 invalid request, -32601 unknown method, -32602 params, -3200N for Jidr codes with `data.code`.
- `server`: NDJSON over a Unix socket (0600, stale-socket cleanup, 104-byte path check), `hello {version}` required first, mismatch → `daemon_stale` (-32008); requests on one connection run concurrently.
- `registry`: providers → command table, duplicate ids rejected; `commands.list`.
- `locks`: per-device single writer; a conflict → `busy` with `{holder, op}`.
- `store`: `devices.json` registry (atomic writes, unique names, AVD id from the name).
- `state`: implements `DaemonContext`, owns the backend, instances and crash watchers, re-adopts running emulators at startup.
- `core_provider`: `daemon.status`, `sdk.status|licenses|accept_license|install`, `image.list|install`, `profile.list`, `device.create|edit|clone|start|stop|reset|delete|list|details|connect_info`, `file.install|push|pull`, `logcat.dump|clear`. `file.flash` is 🔎 (gaps). Long-running commands run inline until jobs land in 4b.
- `jidr-backend::fake::FakeBackend` (gated start, crash trigger) for daemon/client/CLI tests; `Backend` gains `adopt` and `wait_for_crash`.
- Conformance tests over a real socket with the fake backend: bad JSON → -32700, unknown param → -32602 naming it, busy lock, stale daemon, hello required, unknown method, device lifecycle.

## Phase 4a: Daemon core — done (2026-10-13)

Built:
- `jidr-core::provider`: `CommandSpec` (camelCase on the wire), `CommandProvider` (+ `on_booted` hook), `CallCtx`, `DaemonContext`, `DeviceUpdate`, `parse_params`, `DeviceParams`/`NoParams`. Model: `Device.launch: LaunchPrefs`, `ConnectInfo`.
- `jidr-backend`: `Backend::adopt` + `wait_for_crash` on the trait; `fake::FakeBackend` (start gate, crash trigger, launch log, fail-next).
- `jidr-daemon`: `rpc` (codes, `RpcError::from_jidr/to_jidr` with `data.code`/`exit_code`/`details`), `server` (NDJSON over a 0600 Unix socket, stale-socket replace, single instance, 4 MB line cap, `hello` required, concurrent requests per connection), `validate` (JSON-Schema check before locking: unknown/missing keys, primitive types), `locks`, `registry` (duplicate ids rejected), `store` (`devices.json`, runtime state not persisted), `state` (`DaemonContext`, start/stop, crash watchers → `Crashed` without restart, re-adopt on startup, boot hooks, events via broadcast), `core_provider` (23 commands: daemon/sdk/image/profile/device/file/logcat).
- Gaps filed: A1 (B) / A2 (C), to export `provider()`.

Validation `cargo nextest run -p jidr-daemon`: 17 passed. Conformance: bad JSON → -32700 (connection survives), `jsonrpc != 2.0` → -32600, hello required, stale → -32008 `daemon_stale`, unknown method → -32601, unknown/missing/mistyped param → -32602 naming the key, `commands.list` shape, full device lifecycle on the fake backend (create/details/start/connect_info/install/logcat/edit/clone/reset/delete), busy lock with holder `op`, crash → `crashed` with no restart, socket 0600 + single instance + removed on shutdown.

Bug caught by the conformance test: locked commands resolved the device before validating params, so a type error surfaced as `not_found`. Params are now validated against the schema first (`validate::tests`).

Deferred to 4b: long-running commands still run inline (jobs + `{job, pending}` come next), `events.subscribe`, idle exit.

---

## Phase 4b: Jobs, events, lifecycle, client — plan

Gaps: A1/A2 open (B/C), C1 FYI. Nothing blocking anyone.

- Move JSON-RPC types (`rpc`) and `Event` into `jidr-core` so `jidr-client` stays light (no daemon dependency).
- `jobs`: `JobManager` (`Job` + `Cancel` + watch channel per job, last 200 finished kept). A `longRunning` command validates params, takes the device lock and creates the job **before** answering `{job, pending: true}` (so conflicts still fail synchronously); the task holds the lock until it finishes. Progress → `job.progress` events; the end → `job.finished`. Cancel → `cancelled` state (cancel errors recognised via `cancel::is_cancelled`). `JobsProvider`: `jobs.list|get|cancel|wait` (`wait` blocks until terminal or `timeout_s`).
- Events: `events.subscribe {topics}` (prefix filters) per connection → `{"method":"event","params":{topic,data}}` notifications; `logcat.follow {device, filter}` streams `logcat` events on that connection until it closes.
- Lifecycle (D4): `IdleMonitor`: exit once there are no running devices, no clients and no active jobs for `idle_timeout` (10 min; `None` in tests). Tested with paused tokio time.
- `jidr-client`: `Client::connect` (+ hello, stale → `daemon_stale`), `connect_or_spawn` (spawn `jidr daemon` detached: `JIDR_DAEMON_BIN`, the sibling `jidr`, or `PATH`), `call`, `run` (follows jobs to completion), `wait_job`, `subscribe` (broadcast receiver), `cancel_job`.
- Tests: cancel a gated boot → job `cancelled`, device `stopped`, nothing running; a subscriber sees `device.boot` stages + `job.finished`; idle exit with paused time; client end-to-end against an in-process daemon (calls, jobs, events, stale).

## Phase 4b: Jobs, events, lifecycle, client — done (2026-10-13)

Built:
- `jidr-core::rpc` (moved from the daemon) + `Event`, `EVENT_METHOD`.
- `jidr-daemon`: `jobs` (`JobManager` with watch channels, cap of 200 finished, `JobsProvider`: `jobs.list|get|cancel|wait`); long-running commands validate → lock → create job → answer `{job, pending: true}`, the job task holds the lock; `job.started|progress|finished` events. `events.subscribe {topics}` (prefix filters, re-subscribe replaces, lag notice) and `logcat.follow` as per-connection streams. `daemon.shutdown` (allowed before hello) to replace a stale daemon; open connections close when serving ends. `lifecycle::idle_for` (D4: no devices, clients or jobs for 10 min → exit; `DaemonConfig.idle_timeout`). `testing::TestDaemon` + `fake_studio`, shared by daemon/client/CLI/MCP tests. `DaemonBuilder::version` (stale tests).
- `jidr-client`: `Client::connect` (+hello), `connect_or_spawn` (spawns `jidr daemon` in its own process group, logs to `daemon.log`, `restart_stale` path), `call`, `call_as`, `run` (follows jobs, reports progress, maps failed/cancelled), `wait_job`, `cancel_job`, `subscribe`, `events`, `shutdown_daemon`, `pending_job`.

Validation `cargo nextest run -p jidr-daemon -p jidr-client`: 30 passed (3 runs). Covered: cancelling a gated boot → job `cancelled`, device `stopped`, nothing running, lock released; a subscriber gets `spawned → grpc_ready → adb_online → booted` then `job.finished`; topic filters; logcat streaming; failed job carries `boot_timeout`; idle exit after 10 virtual minutes with paused time (activity resets the clock); client calls/jobs/errors/events, stale daemon detected and shut down, spawn path invokes `jidr daemon`, closed daemon → calls fail instead of hanging.

Notes: the spawn test polls for its marker (the spawned process can lag under parallel load).
