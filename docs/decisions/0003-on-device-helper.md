# ADR-0003: A rooted on-device helper for privileged widgets

- **Status:** Accepted (2026-11-13, D44). Proposed 2026-11-12.
- **Issue:** #26 (gap G3, design opportunity O2). First users: #17 (packet loss, bad DNS), then #18 (Wi-Fi signal), #19 (IMEI and device ID), operator/SIM identity, disk limits and gamepad.
- **Minimal build in the same pull request:** the `jidr-helper` crate, `cargo xtask helper`, `root.helper.status`, and `network.shaping.get` / `network.shaping.set {loss_pct}` (the packet-loss half of #17).

## Context

Several Genymotion features Jidr lacks need code that runs as root inside the guest: Linux traffic control for packet loss, a process that holds `/dev/uinput` open for a gamepad, telephony state for operator and IMEI. Today a widget that needs root runs one-off `adb shell` command strings. That doesn't scale:

- every feature quotes its own shell lines, and the result is text the daemon parses;
- nothing can stay alive between calls (a uinput device, an `ifb` redirect, a watchdog that re-applies state);
- each feature that did need a process would ship its own privileged daemon, which is more to secure, version and audit.

Root is already a device property (`root_mode: none | adb | magisk`, see [root.md](../root.md)). Under `adb` the adb shell is root; under `magisk` the shell is granted `su` automatically (D22).

## Decision

One small on-device program, **`jidr-helper`**, runs as root on the guest and answers a typed request/response protocol. The daemon proxies to it: a protocol command such as `network.shaping.set` resolves the device, makes sure the helper runs, and calls one typed method on it. GUI, CLI and MCP see ordinary protocol commands and never talk to the helper.

### 1. Language and build

- **Rust**, in a new Apache-2.0 crate `crates/jidr-helper`: the wire types (`rpc`), the request loop (`server`, independent of the socket so it runs and is tested on any host), the feature code (`netem`), and the daemon's client behind the `host` feature.
- **Targets:** `aarch64-unknown-linux-musl` (Apple Silicon guests) and `x86_64-unknown-linux-musl` (the Linux KVM CI images, D28). Both are **static** executables, so they don't depend on the guest's bionic or its linker, and run on every API level Jidr supports (29–36, D23).
- **No Android NDK.** The binary is linked with the toolchain's own `rust-lld` against Rust's self-contained musl. `cargo xtask helper [--arch …]` adds the rustup targets when missing, sets the linker and the library path `rust-lld` needs on macOS, and writes `target/jidr-helper/<arch>/jidr-helper`. A `helper` Cargo profile (`opt-level = "s"`, symbols stripped, `panic = "abort"`) keeps it small: about 600 KB (arm64) and 680 KB (x86_64).
- **Dependencies on the guest side:** `std`, `serde`, `serde_json`, `sha2`, `hex` and `rustix` (safe `getuid`, `setsid`, `SO_PEERCRED` and `kill`, so the crate needs no `unsafe`). The host-only dependencies (`tokio`, `jidr-core`, `jidr-adb`) are behind `host` and never reach the guest build.
- **CI:** the `check` job lints and tests the crate on Linux, which compiles the guest code (it is `cfg(target_os = "linux")`); `clippy (macOS)` compiles the host stub. The emulator job runs `cargo xtask helper --arch x86_64` before its tests. `cargo xtask e2e-mac` builds the arm64 helper first.

### 2. Shipping

- `cargo xtask release` builds the arm64 helper and puts it in `Jidr.app/Contents/Resources/helper/aarch64/jidr-helper` and in the MCP bundle at `server/helper/aarch64/jidr-helper`. Only arm64 ships, since Apple Silicon Macs run arm64 guests (D15).
- The daemon looks for `<dir>/<arch>/jidr-helper` in: `$JIDR_HELPER_DIR`; `../Resources/helper` next to `jidr` (the app); `helper` next to `jidr` (the MCP bundle); and `target/jidr-helper` (development builds). A missing build is `not_found` and says to run `cargo xtask helper`.
- **The release scan changes:** it rejects any ELF in the bundle today ("Android/Linux code has no place in the macOS app"). It now allows exactly one kind: Jidr's own helper at `helper/<arch>/jidr-helper`. Any other ELF, or a helper anywhere else, still fails the scan.

### 3. Lifecycle: on demand, stateless, never installed

- The daemon pushes the binary to `/data/local/tmp/jidr-helper` (as `.new`, then `chmod 755` and `mv`), and runs `jidr-helper start` **as root**: plainly in the adb shell under `adb` mode, and through `su -c` under `magisk`. `none` is refused with `usage` before anything is pushed.
- `start` is idempotent. If a helper of the same build already answers, it is kept. An older build is asked to `shutdown`; one that holds the socket without answering is killed. Then `start` spawns `jidr-helper serve` in its own session (`setsid`, its own process group, null stdio, so the end of the adb shell doesn't hang it up) and waits until it answers, up to 5 s. Two daemon calls starting it at once both succeed.
- **No bookkeeping in the daemon.** Every call finds the device's existing forward (`adb forward --list`) or makes one, and pings. Only when nothing answers, or a different build or protocol does, does it push and start. This covers every way the helper can disappear:
  - **reboot or cold boot:** the process is gone; the next call starts it again;
  - **snapshot restore:** a snapshot holds the guest's memory, so it restores whatever ran when it was saved: the same helper, an older one, or none. The ping tells which;
  - **daemon restart:** the forward lives in the adb server; the next call reuses it.
- **Nothing is installed on the device.** No `init` service, no Magisk module, no boot script. Reverting root (or a different mode) leaves at most an idle binary in `/data/local/tmp`, which `adb` mode or a wipe removes like any other file.
- **State belongs to the widget, not the helper.** The helper changes the kernel (a `tc` qdisc) and reports it; it remembers nothing. A widget that must survive a cold boot stores its setting on the device record and re-applies it from its provider's `on_booted` hook, the way `root.auto_grant` already does. The minimal `network.shaping.set` doesn't persist yet: a snapshot keeps the loss (it is kernel state), a cold boot clears it. Persisting it is part of #17 proper.

### 4. Transport and protocol

- **On the guest:** the abstract Unix socket `@jidr-helper`. Abstract sockets have no file and no port: nothing on the guest's network (or the host's LAN) can reach it.
- **From the daemon:** `adb forward tcp:0 localabstract:jidr-helper`. adb picks a free port on the host's loopback (D9's dynamic ports), and the daemon connects to `127.0.0.1:<port>`.
- **Framing:** one JSON object per line each way (NDJSON), like the control protocol, without the JSON-RPC envelope:

  ```text
  → {"id":1,"method":"net.shaping.set","params":{"loss_pct":10}}
  ← {"id":1,"result":{"interfaces":[{"name":"wlan0","qdisc":"netem","loss_pct":10.0}]}}
  ← {"id":2,"error":{"code":"unsupported","message":"this image's kernel has no netem (CONFIG_NET_SCH_NETEM)"}}
  ```

- **Typed:** each method is a params struct with a `Method` impl naming its result type, shared by both sides (`jidr_helper::rpc`). The helper exposes only fixed, typed methods. **There is no "run this shell command" method**: that would make the helper a second, unaudited `adb shell`.
- **Errors:** `usage` (bad params, unknown method), `unsupported` (the guest can't do it, e.g. a kernel without netem) and `failed` (a tool failed). The daemon maps `usage` and `unsupported` to Jidr's `usage` (exit 2), `failed` to `unexpected`, and keeps the helper's code in `error.data.helper_error`. A helper that can't be started, doesn't run as uid 0, or can't be reached through the forward is `root_failed` (exit 7).
- **Methods today:** `ping`, `shutdown`, `net.shaping.get`, `net.shaping.set`.

### 5. Security

- **Who can connect:** the helper reads each connection's peer credentials (`SO_PEERCRED`) and serves only uid 0 (root) and uid 2000 (shell). Every `adb forward` connection arrives from adbd, which is one of the two. Apps (uid 10000 and up) are closed unanswered. Anything that can already run as shell or root can already do what the helper does through `adb shell`, so the helper adds no privilege an adb user didn't have.
- **No open port on the device network.** The guest side is an abstract socket. The only TCP port is the forward on the **host's** loopback, opened and owned by the adb server, which is the same exposure as the adb server's own port 5037.
- **SELinux:** the helper runs in the domain of the root shell that starts it (`u:r:su:s0` under `adb root`, `u:r:magisk:s0` under Magisk); `ping` reports it. Jidr adds no SELinux policy.
- **Input:** params are typed and checked (interface names against the kernel's rules, `loss_pct` within 0–100). Tools are run by absolute path (`/system/bin/tc`, `/system/bin/ip`) with argument vectors, never through a shell.
- **What runs is what Jidr ships.** `ping` returns the SHA-256 of the helper's own executable. The daemon keeps a helper only when that matches the build it would push, and replaces it otherwise.

### 6. Versioning

- The helper's `ping` reports its crate `version`, its `protocol` (an integer, `1` today) and its `sha256`.
- The daemon requires the **exact build** it ships (the SHA-256), not just a compatible protocol. Since the daemon pushes the helper itself, there is no reason to talk to any other build, and this avoids a compatibility matrix.
- `PROTOCOL` is bumped on any breaking change to a method's params or result. **`ping` and `shutdown` never change shape**, so any daemon can recognise and replace any older helper.
- The helper protocol is internal and not part of the frozen control protocol (`protocol-v0.1`). The **commands** that use it (`network.shaping.*`, `root.helper.status`) are additive control-protocol commands, logged in its changelog.

### 7. Licensing

- `crates/jidr-helper` is **Apache-2.0**, like the daemon. It depends on no GPL code and on no Slint crate; `cargo xtask license-check` covers it like every other Apache crate.
- The static binary includes musl libc (MIT) from Rust's `*-linux-musl` targets; NOTICE lists it, since the binary is redistributed.
- Clean room: the helper drives Linux's documented `tc`/`netem` and `ip` tools and the kernel's socket API. No Genymotion code or binaries were looked at, and no GPL code was copied (see [clean-room.md](../clean-room.md)).

## Alternatives rejected

| Alternative | Why not |
|---|---|
| Keep one-off `adb shell` strings per widget (today) | No process can live between calls (uinput, `ifb`, watchdogs), every widget hand-quotes shell lines and parses text, and there is no typed contract to test offline. |
| One privileged daemon per feature | What #26 exists to avoid: N binaries to build, start, secure, version and audit. |
| A helper run once per call (`adb shell su -c 'jidr-helper call <json>'`) | Simpler, but can't hold state (a gamepad's uinput device must stay open), pays an `adb shell` and `su` round trip per call, and needs JSON on a shell command line. The long-running helper keeps this as an internal detail: `start` is exactly such a one-shot call. |
| A Java/Kotlin helper (an APK, or a dex run with `app_process`) | Needs `d8` and the Android build tools at build time, and a JVM process (tens of MB) on the guest. Some later features (operator/SIM, IMEI) may need Java-only system APIs; the Rust helper can start an `app_process` dex for those when the time comes, without changing the transport. |
| The NDK target `aarch64-linux-android` | Needs the Android NDK (about 1 GB) on every contributor's Mac and in CI. Static musl binaries run on the Android kernel with no NDK. |
| A Magisk module, `service.d` script or `init` service that starts it at boot | Works under Magisk only (not `adb` mode), re-patches or edits the device, and keeps running after root is reverted. On-demand start covers both modes with nothing installed. |
| A TCP port inside the guest (`adb forward tcp:X tcp:Y`) | Reachable by every app on the guest's network stack, and TCP has no peer uid to check. |
| A host-side Unix socket (`adb forward localfilesystem:…`) instead of a loopback port | Would avoid the loopback port, but adds a socket path to manage per device and is untested with macOS's adb. The loopback port is no more exposed than adb's own; this can change later without touching the helper. |
| JSON-RPC 2.0 exactly, or gRPC/protobuf | The envelope adds nothing on a private, single-client link, and protobuf would need codegen for the guest. NDJSON lines with typed Rust structs on both sides keep the binary small. |
| Accept any helper with the same `protocol` | A compatibility matrix nobody needs: the daemon always has its own build at hand and pushes it. |

## Consequences

- A privileged widget is one helper method plus one provider command. `network.shaping.*` holds only what needs root in the guest: packet loss now, bad DNS later (#17). Speed and delay stay in `network.set`, which the emulator console already does without root.
- netem must be in the image's kernel. API 31 and later have it; API 30's x86_64 emulator kernel doesn't, and `tc` answers ENOENT there. The helper reports a kernel without netem as `unsupported` (a `usage` error at the control protocol) and changes nothing, so packet loss needs API 31 or later.
- Packet loss persists across cold boots in #17 itself: the widget stores it on the device and re-applies it in `on_booted`, like the proxy and `root.auto_grant`. This minimal build doesn't persist it yet.
- The release bundle contains one ELF file, and the scan's rule changes to allow exactly that file.
- `cargo xtask helper` is needed before the helper's emulator tests and before running a privileged widget from a development build; CI and `e2e-mac` do it.
- Commands that use the helper need Shell or Full root. On Play Store images that means Full root, after the one-time guided tap (ADR-0002).
- The first call on a device pushes about 600 KB and starts the helper; later calls cost one `adb forward --list` and a ping.

## Measurements

On 2026-11-12, on a throwaway AVD (D38): API 35 `google_apis` arm64-v8a, kernel `6.6.30-android15`, `userdebug`, SELinux enforcing, Shell root (`adb root`). M1 Pro, with the host under heavy load from other work (load average around 50).

| Check | Result |
|---|---|
| Static musl arm64 build runs on the guest | `jidr-helper --version` → `0.4.0 (protocol 1)`; 596 KB pushed in 0.04 s |
| `jidr-helper start` as root | Returns the `ping` answer at once; `jidr-helper serve` runs as uid 0 in `u:r:su:s0` |
| `adb forward tcp:0 localabstract:jidr-helper` | Answered through adbd (SELinux allows it); `ping` 220–400 ms under that load |
| Kernel support | `CONFIG_NET_SCH_NETEM=y`, `CONFIG_IFB=y` |
| Real packet loss | `net.shaping.set {interfaces: [lo], loss_pct: 30}` → 58 of 100 pings lost (each round trip crosses `lo` twice, so about 51% is expected); `loss_pct: 0` → `noqueue` again, 0 of 50 lost |
| Peer check | `jidr-helper ping` as uid 2000 is answered; as uid 10077 (an app's uid) the connection is reset |
| `start` is idempotent | Run again with the same build: same pid |
| Replacing a build | A build with another SHA-256 shuts the running one down and takes over, and back again |
| Typed errors | `"lo; reboot"` → `usage`; an unknown method → `usage` |
| Interfaces | Before Wi-Fi is up, only `dummy0` has a default route (Android adds one at boot); it is now excluded |

Not verified on a device yet:

- **Shaping `wlan0` on a fully booted guest.** On that host, `system_server` was killed by its own watchdog during boot, so Wi-Fi never came up (the sec-tester notes saw the same at load 140). `helper_emulator.rs` checks it on CI's x86_64 images (API 30, 33, 35), and `cargo xtask e2e-mac` on arm64.
- **Magisk mode** (`su -c`, the `u:r:magisk:s0` domain, and adbd reaching a socket in that domain). The offline tests cover the start command; a run on a Full-root device is still to do.

## Decisions (2026-11-13)

The maintainer accepted the ADR with these answers (D44):

1. **Ship the arm64 ELF in `Jidr.app` and the MCP bundle.** The release scan allows only `helper/<arch>/jidr-helper`. Check with `cargo xtask release --dry-run` that signing and notarization accept it before the next release.
2. **`network.shaping.*`, for loss and bad DNS only.** Speed and delay stay in `network.set`, so the two commands don't overlap.
3. **Packet loss persists across cold boots**, in #17 (stored on the device, re-applied in `on_booted`), not in this minimal build.
4. **Kernels without netem answer `unsupported`.** Packet loss needs API 31 or later; on API 30 the command fails as a `usage` error and changes nothing.
