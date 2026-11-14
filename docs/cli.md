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
| `device start <names…> \| --all [--cold-boot] [--no-wait] [--timeout s] [--force]` | Start and wait for boot. `--timeout` cancels the boot and exits 5. `--no-wait` returns the boot job id. Past the host RAM budget it exits 9 unless `--force`. |
| `device stop <names…> \| --all` | Stop, saving quick-boot state (D29). Flushes the guest's storage (`sync`) first, so settings changed just before the stop survive the next start. |
| `device hibernate <names…> \| --all` | Save each running device's whole state under the snapshot `jidr-hibernate`, check it holds memory, then stop it without the quick-boot save (D42). It uses no RAM; `device start` resumes it in seconds. `device stop` leaves it hibernated; `device reset` discards the state. Auto-hibernate after idle minutes: `settings set --auto-hibernate-minutes N` (off by default) or per device `device edit <name> --auto-hibernate N` (`0` = never, `--auto-hibernate-default` follows the setting). Idle means no Jidr command for the device and no player attached. |
| `device list`, `device details <name>`, `device connect-info <name>` | Status, settings and disk usage; the gRPC endpoint/token and adb serial of a running device. |
| `device clone <name> <new> [--keep-snapshot snap]`, `device edit <name> [--rename n] [--ram …] [--agent-owned bool] [--auto-hibernate N \| --auto-hibernate-default]` | Copy or change a stopped device. With `--keep-snapshot` the copy is a copy-on-write clone (APFS / reflink; a full copy after a disk check elsewhere) that keeps that snapshot and resumes it on its first start, in seconds (D42); without it the copy cold-boots. |
| `device reset <name> --yes`, `device delete <names…> \| --all --yes` | Destructive: wipe data, or delete. Without `--yes` they ask on a terminal and refuse otherwise. |
| `install <name> <apk> [--no-replace] [--grant] [--downgrade]` | Install an APK. A signature mismatch fails with `details.signature_mismatch = true`. |
| `push <name> <local> [remote]`, `pull <name> <remote> <local>` | Copy files (push defaults to `/sdcard/Download/`). |
| `logcat <name> [-n N] [--filter spec…] [-f] [--clear]` | Dump, follow (`-f`) or clear the log. |
| `flash <name> <zip>` | Flash a zip (`file.flash`, ADR-0004). A Magisk or Zygisk module installs on a running Full-root device, which reboots; Jidr then checks that Magisk loaded it. Recovery (TWRP, GApps), Riru and OTA/factory image zips are refused with exit 2 and the reason; a module zip with `module.prop` in a subfolder or an invalid id is refused with the fix. A module on a device without Full root, and any other zip, is copied to `/sdcard/Download`. See [root.md](root.md#flashable-zips). |
| `image list [--installed]`, `image install <img> [--accept-licenses]` | System images. |
| `profile list` | Built-in hardware profiles (D24). `agent-lean` (720×1280, 2 GB, 2 cores, no audio or cameras) costs ~4 GB of host RAM instead of ~7.6 GB. |
| `budget` | Host RAM, the device budget (`settings set --budget-percent`, 75% by default) and what each running device is expected to use and uses now (D42). |
| `sdk status`, `sdk licenses <pkgs…>`, `sdk install <pkgs…> [--accept-licenses]` | The SDK Jidr uses (its private one; Android Studio's is only read, D3). |
| `root set <name> none\|adb\|magisk`, `root verify <name>`, `root status <name>`, `root auto-grant <name> on\|off` | Root modes (provided by the root engine). `auto-grant` grants every app root without a prompt on a Full-root device (D22, off by default; `root.auto_grant`). |
| `root helper <name>` | Start the rooted on-device helper if needed and show its version, uid and SELinux context (`root.helper.status`, ADR-0003). Needs root mode `adb` or `magisk`. |
| `root writable-system <name> on\|off` | A writable `/system` on an `adb`-root device (`-writable-system` + `adb remount`); `off` goes back to the read-only stock image. Cold boots a running device (D29); refused with exit code 2 on `none`/`magisk` devices, Play Store images and images older than API 31. See docs/root.md. |
| `snapshot list\|save\|load\|delete <name> [snapshot]` | Snapshots tagged with their root mode (provided by the widgets provider). |
| `base create <device> <name>`, `base ls`, `base rm <name> --yes` | Bases (D42): `create` saves a running device (any root mode; a Full-root source must pass `su -c id`, see [root.md](root.md#magisk-bases)) as a read-only base with snapshot `jidr-base`, animations off, and stops the device. A base can't be started; `rm` refuses while clones of it are leased. |
| `pool get <base> [--ttl 2h] [--timeout s]`, `pool release <lease\|device>`, `pool renew <lease> [--ttl 2h]`, `pool ls` | Throwaway clones of a base for agents and CI (D42): `get` makes a copy-on-write clone, resumes the base snapshot (seconds) and prints its serial; the lease lasts `--ttl` (default 2 h), then the daemon stops and deletes the clone. A full RAM budget makes `get` wait up to `--timeout` (120 s), then exit 9. The daemon stays up while leases exist (D4). |
| `cert install <name> <file>`, `cert list <name>`, `cert verify <name>`, `cert remove <name> <cert>\|--all` | CA certificates in a Full-root device's system store (root engine, `root.cacert.*`); install and remove restart the device. See docs/root.md. |
| `proxy set <name> [host:]port`, `proxy clear <name>`, `proxy show <name>` | The device's global HTTP proxy (widgets provider, `network.proxy.*`). The host defaults to `10.0.2.2`, this Mac. |
| `network loss <name> [percent]` | Drop `percent` % of the device's outgoing packets on Wi-Fi and mobile data (`0` removes it), or show the current loss without `percent` (`network.shaping.*`, through the on-device helper). Needs root mode `adb` or `magisk` and API 31 or later (API 30's kernel has no netem: exit 2); a cold boot clears it. See docs/root.md. |
| `audio inject <name> <file> [--loop] [--rate Hz [--channels 1\|2]] [--no-wait]`, `audio stop <name>` | Play a WAV as the device's microphone (widgets provider, `audio.*`); `--rate` reads the file as headerless 16-bit PCM. See [Microphone](#microphone). |
| `jobs list [--all]`, `jobs wait <job>`, `jobs cancel <job>` | Long-running operations. |
| `commands` | Every protocol command (what the GUI and MCP see). |
| `call <method> [params-json]` | Call any protocol command directly, e.g. `jidr call device.start '{"device":"Phone","cold_boot":true}'`. Follows jobs. |
| `daemon [run]`, `daemon stop`, `daemon status` | Run the daemon in the foreground, stop it (devices keep running), or show it. |
| `shell [script]` | Interactive shell, or run a script of commands; see [Shell](#shell). |
| `completions <shell>` | Shell completions. |

Global flags: `--json`, `-q/--quiet` (no progress), `--restart-stale` (or `JIDR_RESTART_STALE=1`).

## Shell

`jidr shell` is an interactive shell, Jidr's take on Genymotion Shell. Each line is a `jidr` command without the `jidr` (a leading `jidr` is ignored, so pasted lines work). It uses the CLI's own command definitions over one daemon connection, so every command, flag, output and exit code is the same as in the one-shot CLI, and it adds nothing to the protocol.

```text
$ jidr shell
jidr> use Phone
Using Phone.
jidr (Phone)> device start --timeout 300
jidr (Phone)> proxy set 8080
jidr (Phone)> snapshot save before-login
jidr (Phone)> call sensor.rotate '{"angle":90}'
jidr (Phone)> --json device details
jidr (Phone)> exit
```

| Builtin | What it does |
|---|---|
| `use <device>` | Make it the current device, shown in the prompt; `use` alone clears it. |
| `source <file>` | Run a script (below) inside the shell. |
| `help [command]` | The builtins and commands, or a command's `--help`. |
| `exit [code]`, `quit`, Ctrl-D | Leave the shell. |

**The current device** is filled in for commands that take a device first (`device start`, `proxy set`, `snapshot save`, `install`, `logcat`, `root`, `cert` …) when the line doesn't name one: if the first argument is an existing device's name or id it is used as it is, otherwise the current device is put in front of the arguments (`proxy set 8080` runs `proxy set Phone 8080`). Lines with `--all` are left alone. `call <method> [params]` gets `"device"` added to its params when the method takes one and the params don't name it.

**Words** split like a POSIX shell without expansion: `'…'` is literal, `"…"` honours `\"` and `\\`, `\` escapes one character, and `#` at the start of a word begins a comment. There are no variables, pipes or `$(…)`; use your own shell for those.

**Completion and history.** Tab completes builtins, subcommands, flags, value choices (`root set Phone <Tab>`), device names, protocol methods from `commands.list` after `call`, and file paths. History is kept in `shell_history` in Jidr's data directory (`~/Library/Application Support/Jidr`, or `$JIDR_HOME/data`). Ctrl-C stops the running command (a job it started keeps running in the daemon, as when Ctrl-C ends a one-shot `jidr`; `jobs cancel` stops it) and the shell carries on.

**Scripts.** `jidr shell setup.jidr`, piped input (`jidr shell < setup.jidr`, or `-` for stdin) and `source` run the lines in order, skipping blank lines and `#` comments, so `#!/usr/bin/env -S jidr shell` works as a first line. The first failing line stops the script: stderr says `setup.jidr:5: stopped here (exit code 3)` and `jidr shell` exits with that command's [exit code](#exit-codes). `exit <code>` ends a script early. `--json` and `-q` given before `shell` apply to every line; with `--json`, each command prints its own envelope line.

```bash
# setup.jidr
device create Phone --image 35 --profile phone-6.1-fhd
use Phone
device start --timeout 300
proxy set 8080
snapshot save clean
```

## HTTP proxy

Point a running device at Burp, mitmproxy or any HTTP proxy on your Mac:

```bash
jidr proxy set Phone 8080             # 10.0.2.2:8080, the Mac's port 8080
jidr proxy set Phone 192.168.1.5:3128 # a proxy elsewhere on the network
jidr proxy show Phone
jidr proxy clear Phone
```

Inside the emulator `10.0.2.2` is the Mac's `127.0.0.1`, so Burp's default listener (`127.0.0.1:8080`) works as it is; `127.0.0.1` on the device is the device itself. The proxy is Android's global `http_proxy` setting: apps that use the system proxy (most HTTP stacks, WebView, Chrome) send their traffic through it, on Wi-Fi and mobile data. It takes effect at once and stays across restarts and snapshots until you clear it. To read HTTPS, apps must also trust the proxy's CA as a system certificate: `jidr cert install` on a Full-root device (see [CA certificates](#ca-certificates)).

## CA certificates

To read HTTPS from apps, trust the proxy's CA as a system certificate on a Full-root (Magisk) device:

```bash
jidr root set Phone magisk                   # once
jidr cert install Phone ~/Downloads/cacert.der   # Burp's CA (PEM or DER); restarts the device
jidr cert list Phone
jidr cert remove Phone a4ac454c.0            # or --all
```

`install` restarts the device and fails with exit code 7 (root failed) unless the certificate ends up in the store apps read. How it works on each Android version is in docs/root.md.

## Microphone

Play an audio file into a running device's microphone, for voice input, speech recognition or call tests:

```bash
jidr audio inject Phone ~/Desktop/hello.wav           # waits until it has played
jidr audio inject Phone ~/Desktop/hello.wav --loop --no-wait
jidr audio stop Phone
jidr audio inject Phone capture.pcm --rate 16000      # headerless 16-bit little-endian PCM, mono
```

Jidr streams the file over the emulator's gRPC `injectAudio`. WAV files can be 8- or 16-bit PCM (sent as they are), or 24/32-bit PCM or 32/64-bit float (converted to 16-bit), mono or stereo, at 8–48 kHz. Other files, other rates and more channels exit 2; convert them first, e.g. `afconvert -f WAVE -d LEI16@44100 in.m4a out.wav`. The audio plays as the device reads its microphone: start the app's recording before or after, and if nothing records, playback waits (the progress stops) until something does or you stop it. `--loop` repeats until `audio stop`; `jobs cancel` also ends it. One file plays at a time per device; a second `audio inject` exits 4. The Mac's own microphone isn't routed to the device (yet).

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
| 9 | Resource exhausted: starting would go past the host RAM budget (`error.data.devices` lists what uses it; see `jidr budget`) |

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
