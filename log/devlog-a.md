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

---

## Phase 4c: Protocol v0.1 freeze — plan

Gaps: A1/A2 open (B/C, not blocking), C1 FYI.

- `jidr_daemon::protocol_doc::render(&[CommandSpec])`: per-command Markdown (label, flags, params table from the JSON Schema with types, required markers and doc-comment descriptions).
- `docs/control-protocol.md`: transport, framing, hello/stale, error codes, jobs, events and topics, connection-scoped methods, commands (the generated section between `<!-- commands:begin/end -->` markers), changelog. A test fails when the generated section drifts from `commands.list`; `JIDR_UPDATE_DOCS=1` rewrites it.
- `tests/fuzz_params.rs`: 10,000 proptest-generated cases through the real socket: random raw lines, structurally broken requests, and requests for every real method with random JSON params. Each must get exactly one well-formed JSON-RPC answer with the right id (a panic shows up as a missing answer); the daemon still answers `daemon.status` at the end. Plus a direct fuzz of `validate::validate` against every command schema.
- Then commit and tag `protocol-v0.1`.

## Phase 4c: Protocol v0.1 freeze — done (2026-10-13)

Built:
- `docs/control-protocol.md`: transport, handshake, errors table, locks, jobs, events and topics, connection-scoped methods, generated command reference (`protocol_doc::render` from `commands.list`, kept in sync by `tests/protocol_doc.rs`; `JIDR_UPDATE_DOCS=1` rewrites it), changelog.
- `tests/fuzz_params.rs`: 10,000 deterministic proptest cases through the socket (garbage lines, non-request JSON, wrong `jsonrpc`, missing method, unknown methods, every real method incl. connection-scoped ones with schema-shaped random params). Every case gets exactly one well-formed answer with the right id; daemon healthy afterwards (3.3 s). Plus 2,000 cases of `validate` against every schema.
- `DaemonConfig::new` (product defaults) and `DaemonConfig.sdk_repo`; `TestDaemon` uses an empty `file://` repository so no test touches the network.
- HTTP timeouts in `Fetcher` (connect 15 s, read 60 s, listing 60 s).

Validation `cargo nextest run -p jidr-daemon && git rev-parse protocol-v0.1`: passes after tagging.

Finding: the first fuzz run hung on a case. The test daemon was fetching Google's live listings (`image.list`, `sdk.*`) with no HTTP timeout. Fixed both: tests run offline, and the fetcher can no longer hang forever.

Tag `protocol-v0.1` created on this commit: streams B and C are unblocked (B also needs ADR-0001, done in Phase 1).

---

## Phase 5a: CLI — plan

Gaps: A1/A2 open (B/C), C1 FYI; nothing blocking. Tag `protocol-v0.1` done; Stream C notified, Stream B proposed.

`apps/jidr` (hand-written clap derive over `jidr-client`):
- Globals: `--json` (versioned envelope `{"jidr_json": 1, "ok": …, "result"|"error": …}`), `-q/--quiet`, `--restart-stale` (`JIDR_RESTART_STALE`; else a TTY prompt, else exit 8).
- `daemon [run|stop|status]`: `run` serves with `AospBackend` + `DaemonConfig::new` (idle exit D4), logs to stderr (`JIDR_LOG`).
- `device create|start|stop|list|details|clone|reset|delete|edit|connect-info` (`start/stop/delete` take several names or `--all`; `start` waits for boot by default, `--no-wait`, `--timeout` cancels the job and exits 5; destructive ones need `--yes` off a TTY).
- `install|push|pull|logcat [--follow|--clear|--lines|--filter]`; `flash` → usage error pointing at the 🔎 gap.
- `image list|install`, `profile list`, `sdk status|licenses|install` with `--accept-licenses` (prints each license id + SHA-256, records consent, retries; a TTY asks; otherwise exit 6 listing what to accept).
- `root set|verify|status`, `snapshot list|save|load|delete`: thin calls to Stream C/B commands ("not available in this build" until registered).
- `jobs list|wait|cancel`, `commands` (lists `commands.list`), `call <method> [params]` (raw protocol access: full parity with GUI/MCP), `completions <shell>`.
- Exit codes = Jidr error codes (clap usage errors are already 2).
- Tests: `assert_cmd` against `TestDaemon` (fake backend) via `JIDR_HOME`: JSON envelopes, exit codes 2/3/4/6, lifecycle, `--all`, install/logcat, `--timeout`, flash, completions, `call`.
- `scripts/e2e-cli.sh`: real emulator, temp `JIDR_HOME`: create → start → wait-boot → install → stop → delete → `daemon stop`, all `--json`, non-interactive.
- `docs/cli.md`.

## Phase 5a: CLI — done (2026-10-13)

Built: `apps/jidr` (`cli.rs` clap derive, `output.rs` human tables + versioned JSON envelope, `main.rs`): every command in the plan, plus `commands`, `call` (raw protocol access), `daemon run|stop|status` (`run` = `AospBackend` + `DaemonConfig::new`, SIGINT/SIGTERM shutdown), `--timeout` (cancels the job → exit 5), `--no-wait`, `--all`, `--yes` confirmations, license flow (`--accept-licenses` / TTY prompt / exit 6 with the pending list), stale daemon prompt or `--restart-stale`, thin `root.*`/`snapshot.*` calls with a clear "not available in this build" until providers land. `docs/cli.md`, `scripts/e2e-cli.sh`.

Validation `cargo nextest run -p jidr && scripts/e2e-cli.sh`: 9 tests passed; e2e on a real emulator passed in 48 s (auto-spawned daemon, API 31 arm64 from the read-only Studio SDK, boot 26 s, APK install, logcat, stop, delete, daemon stop).

Bug found (regression test `cli::tests::cli_definition_is_valid`): `logcat` gave `-f` to both `--filter` and `--follow`; clap panics on that at runtime in debug builds. `--filter` has no short form now.

Notes: the e2e APK defaults to an AOSP sample app found in the SDK's images (`JIDR_TEST_APK` to override); CI x86_64 images may need `JIDR_TEST_APK` set.

---

## Phase 5b: Cache, uninstall, logs — plan

Gaps: A1/A2 open; C1 FYI. Will file A3 for C (`root.set` takes `magisk_sha256` from the cache).

- `jidr-sdk::cache::DownloadCache` (D30): files under `cache/downloads/sha256/<hex>[.<ext>]`; `get(sha256)`, `fetch(url, sha256, …)` (download → verify SHA-256 → atomic rename; reuse when present and valid), `import(path, expected)` (`--magisk <path>` offline), `list`, `prune(older_than)`. `Fetcher::download` returns both SHA-1 and SHA-256.
- Daemon commands: `cache.list`, `cache.import {path, sha256?}`, `cache.prune {older_than_days?}`, `device.logs {device, output?}` (zip with the device record, config.ini, emulator + crash logs and a `logcat -d` when running). `device.details` already reports disk usage (D30).
- CLI: `jidr cache list|import|prune`, `jidr device logs <name> [--zip path]`, `jidr root set <dev> magisk --magisk <apk>` (imports into the cache, passes `magisk_sha256`), `jidr uninstall [--purge] [--yes]` (D31): stop devices and the daemon, kill any leftover emulator running a Jidr AVD, remove Jidr's MCP entries (Claude Desktop, Claude Code `~/.claude.json` incl. per-project, Cursor `~/.cursor/mcp.json`; backup `*.jidr-backup` first, key order preserved), remove cache + logs, and with `--purge` the data dir (private SDK, AVDs, registry, consent). Never touches Android Studio's SDK or `~/.android`.
- `scripts/e2e-cli.sh` ends with `uninstall --purge` and checks nothing Jidr-owned remains (no files, no socket, no emulator with a Jidr AVD). `scripts/e2e-studio-untouched.sh` defaults to running `e2e-cli.sh`, so install → use → purge is checked against the Studio fingerprint.

## Phase 5b: Cache, uninstall, logs — done (2026-10-13)

Built:
- `jidr-sdk::cache::DownloadCache` (content-addressed by SHA-256: `fetch` verifies before storing and reuses verified copies offline, `import`, `get_verified` drops corrupt entries, `list`, `prune(older_than)`); `Fetcher::download` returns SHA-1 + SHA-256 (`Digests`).
- Daemon: `cache.list|import|prune`, `device.logs` (zip: device.json, config.ini, hardware-qemu.ini, launch params, emulator + crash logs, logcat when running, about.txt; `bundle.rs`). Disk usage per device was already in `device.details` (D30).
- CLI: `cache list|import|prune`, `device logs <name> [--zip]`, `root set … --magisk <apk>` (imports, passes `magisk_sha256`; gap A3 for C), `uninstall [--purge] --yes` (`uninstall.rs`: stops devices + daemon, kills leftover emulators running Jidr AVDs, removes Jidr's MCP entries from Claude Code (incl. per-project), Claude Desktop and Cursor configs with a `.jidr-backup`, deletes cache + logs (+ data with `--purge`), with a guard that refuses home, `~/Library/Android`, `~/.android` and shallow paths). `serde_json` `preserve_order` so rewritten configs keep their key order.
- `scripts/e2e-cli.sh` now ends with a second running device and `uninstall --purge` under a scratch `HOME`, then checks no data/cache/logs/socket and no live emulator with a Jidr AVD (`scripts/lib/no_jidr_emulators.py`). `scripts/e2e-studio-untouched.sh` defaults to the sdk emulator tests + `e2e-cli.sh`.

Validation `cargo nextest run -p jidr && scripts/e2e-studio-untouched.sh`: 16 CLI tests passed; full real-emulator flow (create → boot 26 s → install → logcat → stop → delete → second device booted → `uninstall --purge`) passed in 93 s, and all 296,062 Studio SDK/AVD files are unchanged.

---

## Phase 6: MCP server — plan

Gaps: A1/A2/A3 open for B/C (not blocking); C1 FYI.

`jidr-mcp` on rmcp 3.5 (hand-written `ServerHandler`, `transport-io` for stdio), `jidr mcp [--allow-destructive]` in `apps/jidr`:
- Generic tools: `command_list {prefix?}` (from `commands.list`), `command_run {command, params, timeout_s}` (follows jobs; a job still running at the timeout is returned as is), `jobs_list {all}`, `jobs_cancel {job}`.
- Typed hot paths: `screenshot {device}` (gRPC `getScreenshot` PNG, fitted in 1024×1024 per ADR-0001: width + height requested; returns the image + text with the screen size and scale), `tap`/`swipe` (coordinates in that screenshot's space by default, `coords: "device"` for raw pixels; `adb input`), `type_text` (gRPC `sendKey` text, optional Enter), `press_key` (Home/Back/AppSwitch/Enter… via gRPC `sendKey`), `ui_dump` (uiautomator XML), `logcat_tail {lines, filter}`.
- Annotations on every tool (read-only / destructive / idempotent / open-world hints).
- D5 policy: a destructive command (`CommandSpec.destructive`) is refused unless the server runs with `--allow-destructive` or the target device is `agent_owned`; `device.create` through MCP marks the device agent-owned unless told otherwise. Connection-scoped methods (`hello`, `events.subscribe`, `logcat.follow`, `daemon.shutdown`) aren't callable.
- Jidr errors come back as tool results with `isError` + structured `{code, message, data}` so agents can act on them.
- Tests: in-process (tokio duplex) rmcp client against `TestDaemon`: tool list + annotations, generic flow, D5 refusals/allowances, blocked methods. `--features emulator`: 5 scripted agent tasks on a real emulator through MCP (boot + screenshot, open Settings from the app drawer via ui_dump/tap, search Settings by typing, logcat_tail, stop + delete under the policy), with per-call latency printed (not gated).
- `docs/mcp.md` with the "Add Jidr to Claude" snippets.

## Phase 6: MCP server — done (2026-10-14)

Built: `jidr-mcp` (rmcp 3.5, hand-written `ServerHandler`): `command_list`, `command_run` (follows jobs, timeout → job), `jobs_list`, `jobs_cancel`, `screenshot` (gRPC PNG fitted in 1024², width+height per ADR-0001), `tap`/`swipe` (screenshot or device coordinates, `adb input`), `type_text` (`adb input text`, shell-escaped), `press_key` (gRPC `sendKey`), `ui_dump`, `logcat_tail`; annotations on every tool; Jidr errors as `isError` results with structured codes; D5 policy (destructive needs `--allow-destructive` or an agent-owned device; `device.create` via MCP is agent-owned by default); connection-scoped methods blocked. `jidr mcp [--allow-destructive]`. `docs/mcp.md`.

Validation `cargo nextest run -p jidr-mcp --features emulator`: 9 passed. Five scripted agent tasks on a real emulator (create+boot+screenshot, open Settings via the app drawer with ui_dump/tap, type into the launcher search, filtered logcat, stop+delete under D5); latency recorded in docs/mcp.md.

Bugs/findings (with regression tests where code changed):
1. `logcat.dump` with a filter + line count came back empty: logcat applies `-t N` to the raw buffer before tag filtering. Now we dump and keep the last N matching lines (`core_provider::tests::filtered_logcat_tails_after_filtering`).
2. gRPC `sendKey` text injection sometimes coincided with the Settings search closing; switched `type_text` to `adb input text` (`device::tests::input_text_is_shell_safe`). Root cause of the closing turned out to be Settings Intelligence being killed during first-boot updates ("app died"), so task 3 now types into the launcher search instead.
3. The launcher's first search shows "Always show keyboard here?"; the agent task answers it. The 2 GB profile starved first boot; tasks use the default 4 GB profile.

Open: `ui_dump` is slow (uiautomator, 4–6 s median); a faster accessibility path could come later.

---

## 2026-10-14: merge of main (D35–D40, gaps research), camera options, CI

- Merged `main` (d47a49b): took main's `docs/gaps.md` and plan, re-added my Part B rows (G1/G5 marked Done with commits; A1–A3 became G6–G8) and the Status lines of Phases 0–6.
- Phase 3b follow-up (R7, D36): `LaunchOptions.camera_back/camera_front` → `-camera-back/-camera-front` (`emulated`, `virtualscene`, `none`, `webcamN`; validated, `jidr_backend::valid_camera`), persisted per device as `Device.launch.camera_back/front` so the Phase 8a webcam widget sets them and restarts. `-gpu host` remains the macOS default (ADR-0001). The backend emulator test now cold-boots with `camera_back = emulated` and checks `hw.camera.back` in the emulator's `hardware-qemu.ini`.
- CI slimmed (`.github/workflows/ci.yml`): `check` (ubuntu-latest) runs the full `cargo xtask ci`; `clippy-macos` (macos-latest, rust-cache, protoc) runs only `cargo clippy --workspace --all-targets -- -D warnings`, skipped via a `changes` job (dorny/paths-filter, `predicate-quantifier: every`) when a PR only touches `docs/`, `site/` or `*.md`; the Linux KVM `emulator` job is unchanged. The repo is public so runners are free, but hosted macOS queues slower and can't boot the emulator.

## 2026-10-14: CI fixes for Stream C's PR (G10, G11) and the emulator leak

- G10: `spikes/display/build.rs` falls back to the vendored protos when the SDK has no emulator (CI runners). G11: the emulator CI job installs `libfontconfig1-dev libxkbcommon-dev` (the workspace build includes Slint). Pushed as fb8d18e.
- Leak report (from Stream C, via a RAM-usage session): 13 orphaned emulators (12 × `agent-phone` from `jidr-mcp`'s agent tasks, 1 × `jidr_resilience`) used ~66 GB on the 16 GB Mac. Cause: my emulator tests had no cleanup on failure, and a killed test process (Ctrl-C, nextest timeout) never runs `Drop`; the in-process daemon's emulators outlive it by design. Fix (`jidr_backend::testing`):
  - `AvdGuard`: kills the AVD's emulator on drop (pass, failure, panic);
  - a per-test-process reaper (`sh` in its own process group) that waits for the test process to die, then kills every registered AVD's emulator, covering SIGKILL/Ctrl-C/timeouts;
  - `kill_orphans(prefix)`: at test start, kills orphaned (ppid 1) qemu of earlier runs;
  - per-process AVD names (`unique_avd`) so cleanup never touches another run;
  - `.config/nextest.toml`: emulator-booting test binaries run one at a time (four in parallel starved the Mac and flaked the UI tasks);
  - `scripts/e2e-cli.sh` cleans up with `uninstall --purge` on any exit, including Ctrl-C.
- Port allocation: `allocate_avoiding` skips console/adb ports that live emulators report in their discovery files (two leaked emulators had both asked for 5554; a 127.0.0.1 bind test can miss a port held elsewhere). Regression test `ports::tests::skips_ports_live_emulators_hold`.
- Verified: a run Ctrl-C'd mid-test and a deliberately failing test (`JIDR_TEST_FAIL_AFTER_BOOT=1`) both leave 0 emulators; the full `jidr-sdk`/`jidr-backend`/`jidr-mcp` emulator suites pass (59 tests) with 0 qemu processes and no reapers afterwards.

## 2026-10-14: PR #2 emulator jobs (agent task 3 on x86_64 images)

- PR #2's emulator jobs (Linux KVM, API 30/33/35, `google_apis;x86_64`) failed in `five_agent_tasks` task 3: the drawer search field was never found ("field found: false" ×3). The x86_64 images run Launcher3, whose drawer search has neither Pixel label the test looked for, and one swipe doesn't always open the drawer. Fix: `find_search_field` falls back to the drawer's `EditText`, then to a node whose `resource-id` contains `search`; task 3 swipes up to three times, as task 2 does, and prints the drawer dump when no field shows. `attr` now also reads a node's first attribute. Regression test `search_field_falls_back_to_the_text_field`. The arm64 API 31 run still passes (105 s).
- `.config/nextest.toml`: the emulator test group filter is `binary(~_emulator) | binary(agent_tasks)`, so it covers Stream C's `jidr-root` `adb_emulator`. An exact `binary(adb_emulator)` fails nextest's config check on branches that lack that binary.

## 2026-10-14: G6, the widgets provider in `jidr daemon`

- `jidr daemon` registered only the core and jobs providers, so the CLI and MCP got `unknown method` for every widget command (`gps.*`, `battery.*`, `snapshot.*`, ...). `run_daemon` now builds the daemon through `build_daemon`, which adds `jidr_widgets::provider()`. `apps/jidr` depends on `jidr-widgets` (Apache-2.0, no Slint, so D18 holds).
- Regression test `tests::daemon_serves_widget_commands` (in `apps/jidr/src/main.rs`): the daemon's registry has `gps.set`, `battery.set` and `snapshot.save`. It fails without the provider line.
- `jidr-root` (G7) is still not registered; `jidr root …` keeps the "aren't available" message.
