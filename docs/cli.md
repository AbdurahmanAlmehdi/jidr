# `jidr` command line

`jidr` drives everything the Jidr app can do, from a terminal or a CI script. It talks to the Jidr daemon, which it starts automatically when needed (D4). Run `jidr --help` or `jidr <command> --help` for every flag; `jidr completions zsh|bash|fish` prints shell completions.

## Quick start

```bash
jidr image list                                   # images for this Mac
jidr image install 35 --accept-licenses           # API 35 google_apis
jidr device create Phone --image 35               # rooted with adb by default (D7)
jidr device start Phone                           # waits until booted
jidr install Phone app.apk
jidr logcat Phone -n 100
jidr device stop Phone
```

In CI, add `--json` and check the exit code:

```bash
jidr --json device start Phone --timeout 300 || exit $?
```

## Commands

| Command | What it does |
|---|---|
| `device create <name> --image <img> [--profile id] [--ram MB] [--cpus N] [--width px] [--height px] [--density dpi] [--data-mb MB] [--root none\|adb\|magisk] [--agent-owned]` | Create a device. `<img>` is `35`, `35:google_apis`, `35:google_apis_playstore`, `35:google_apis:arm64-v8a` or a full `system-images;…` package. |
| `device start <names…> \| --all [--cold-boot] [--no-wait] [--timeout s]` | Start and wait for boot. `--timeout` cancels the boot and exits 5. `--no-wait` returns the boot job id. |
| `device stop <names…> \| --all` | Stop, saving quick-boot state (D29). |
| `device list`, `device details <name>`, `device connect-info <name>` | Status, settings and disk usage; the gRPC endpoint/token and adb serial of a running device. |
| `device clone <name> <new>`, `device edit <name> [--rename n] [--ram …] [--agent-owned bool]` | Copy or change a stopped device. |
| `device reset <name> --yes`, `device delete <names…> \| --all --yes` | Destructive: wipe data, or delete. Without `--yes` they ask on a terminal and refuse otherwise. |
| `install <name> <apk> [--no-replace] [--grant] [--downgrade]` | Install an APK. A signature mismatch fails with `details.signature_mismatch = true`. |
| `push <name> <local> [remote]`, `pull <name> <remote> <local>` | Copy files (push defaults to `/sdcard/Download/`). |
| `logcat <name> [-n N] [--filter spec…] [-f] [--clear]` | Dump, follow (`-f`) or clear the log. |
| `flash <name> <zip>` | Not available yet (🔎 research, see docs/gaps.md). |
| `image list [--installed]`, `image install <img> [--accept-licenses]` | System images. |
| `profile list` | Built-in hardware profiles (D24). |
| `sdk status`, `sdk licenses <pkgs…>`, `sdk install <pkgs…> [--accept-licenses]` | The SDK Jidr uses (its private one; Android Studio's is only read, D3). |
| `root set <name> none\|adb\|magisk`, `root verify <name>`, `root status <name>` | Root modes (provided by the root engine). |
| `snapshot list\|save\|load\|delete <name> [snapshot]` | Snapshots tagged with their root mode (provided by the widgets provider). |
| `jobs list [--all]`, `jobs wait <job>`, `jobs cancel <job>` | Long-running operations. |
| `commands` | Every protocol command (what the GUI and MCP see). |
| `call <method> [params-json]` | Call any protocol command directly, e.g. `jidr call device.start '{"device":"Phone","cold_boot":true}'`. Follows jobs. |
| `daemon [run]`, `daemon stop`, `daemon status` | Run the daemon in the foreground, stop it (devices keep running), or show it. |
| `completions <shell>` | Shell completions. |

Global flags: `--json`, `-q/--quiet` (no progress), `--restart-stale` (or `JIDR_RESTART_STALE=1`).

## Licenses (D8)

Images and the emulator are downloaded from Google on your machine, after you accept their license. On a terminal, `jidr image install` shows each license and asks. Headless, `--accept-licenses` prints each license id and the SHA-256 of its text, then records your consent. Read the texts first with `jidr sdk licenses <package>`. When a license text changes, Jidr asks again.

## JSON output (`--json`)

Every command prints exactly one JSON line (`logcat -f` prints one per log line):

```json
{"jidr_json": 1, "ok": true, "result": …}
{"jidr_json": 1, "ok": false, "error": {"code": "not_found", "exit_code": 3, "message": "no device named \"x\"", "data": null}}
```

`jidr_json` is the envelope version; it changes only on breaking changes (see the changelog below). `result` is the protocol result of the underlying command (docs/control-protocol.md). Commands that act on several devices (`device start|stop|delete`) return an array.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | OK |
| 1 | Unexpected error |
| 2 | Usage: bad arguments or params, unknown command, missing `--yes` |
| 3 | Not found (device, image, job…), or the device isn't running |
| 4 | Busy: another operation holds the device (`error.data.holder` is the job) |
| 5 | Boot timeout |
| 6 | SDK, emulator, download or license problem |
| 7 | Root failed |
| 8 | Daemon stale: a daemon of another version is running (use `--restart-stale`) |

## Environment

| Variable | Effect |
|---|---|
| `JIDR_HOME` | Put all Jidr data under this directory (tests, portable setups) |
| `JIDR_LOG` | Daemon log filter, e.g. `debug` |
| `JIDR_DAEMON_BIN` | The `jidr` binary used to auto-start the daemon |
| `JIDR_SDK_REPO` | SDK repository mirror (default Google's) |
| `JIDR_RESTART_STALE` | Same as `--restart-stale` |

## Changelog

| `jidr_json` | Change |
|---|---|
| 1 | Initial format. |
