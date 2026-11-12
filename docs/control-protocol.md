# Jidr control protocol (v0.1)

The daemon (`jidr daemon`) owns every device. The GUI, the CLI (`jidr`) and the MCP server (`jidr mcp`) are clients of this protocol and nothing else, so anything one client can do, all can. Reference client: `crates/jidr-client`.

Frozen as **v0.1** at the git tag `protocol-v0.1`. A change after the tag needs an entry in the [changelog](#changelog).

## Transport

- **Socket:** a Unix domain socket at `~/Library/Application Support/Jidr/daemon.sock` (`$JIDR_HOME/data/daemon.sock` when `JIDR_HOME` is set), mode `0600`. One daemon per user; a second one refuses to start.
- **Framing:** [JSON-RPC 2.0](https://www.jsonrpc.org/specification), one JSON object per line (newline-delimited JSON, as in MCP's stdio transport). Lines are at most 4 MB.
- **Concurrency:** requests on one connection run concurrently; match answers by `id`. Notifications from the daemon have no `id`.
- **Lifetime (D4):** clients start the daemon on demand (`jidr daemon`, detached). It exits after 10 minutes with no running device, no connected client, no active job and no pool lease. Emulators that outlive a daemon are re-adopted by the next one.

## Handshake

The first request on a connection must be `hello`. Anything else before it gets `-32600`.

```json
→ {"jsonrpc":"2.0","id":1,"method":"hello","params":{"version":"0.1.0","client":"cli"}}
← {"jsonrpc":"2.0","id":1,"result":{"version":"0.1.0","protocol":"0.1","pid":4242}}
```

Clients ship with the daemon, so versions must be equal; there's no negotiation. A mismatch means a stale daemon from an older install:

```json
← {"jsonrpc":"2.0","id":1,"error":{"code":-32008,"message":"the running daemon is 0.1.0 but this client is 0.2.0; restart the daemon","data":{"code":"daemon_stale","exit_code":8,"details":{"daemon_version":"0.1.0","client_version":"0.2.0"}}}}
```

`daemon.shutdown` is the one method allowed before `hello`: it lets a newer client replace a stale daemon. Running emulators keep running and are re-adopted.

## Errors

Protocol errors use the standard JSON-RPC codes. Jidr errors carry one of nine codes, which are also the CLI's exit codes, in `error.data.code`; `error.data.details` holds structured extras.

| `data.code` | JSON-RPC code | CLI exit | Meaning |
|---|---|---|---|
| — | -32700 | 2 | Not valid JSON |
| — | -32600 | 2 | Not a valid request (wrong `jsonrpc`, no `hello` yet) |
| — | -32601 | 2 | Unknown method |
| `usage` | -32602 | 2 | Invalid params; the message names the offending key |
| `unexpected` | -32001 | 1 | A bug, or an error we didn't classify |
| `not_found` | -32003 | 3 | No such device, image, profile, job or snapshot (or the device isn't running) |
| `busy` | -32004 | 4 | Another operation holds the device: `details: {holder, op, device}` |
| `boot_timeout` | -32005 | 5 | The device didn't boot in time |
| `sdk` | -32006 | 6 | SDK, emulator, adb, download or license problem (`details.pending_licenses` lists what to accept) |
| `root_failed` | -32007 | 7 | Rooting, verifying or reverting root failed |
| `daemon_stale` | -32008 | 8 | Client and daemon versions differ |
| `resource_exhausted` | -32009 | 9 | Starting would go past the host RAM budget (D42): `details: {needed_mb, used_mb, budget_mb, total_mb, devices: [{id, name, expected_mb, measured_mb?}], suggestions}`. `device.start {force: true}` overrides it |

Params are checked against the command's JSON Schema before anything runs: unknown keys, missing required keys and wrong primitive types are all `-32602`.

## Devices and locks

Commands take a device as `"device": "<name or id>"`; names are unique, case-insensitive. Commands marked *locks the device* take a per-device single-writer lock. A conflicting call fails immediately with `busy`; it doesn't queue. Reads never lock.

## Jobs

Commands marked *long-running* don't block. After validation and locking they answer:

```json
← {"jsonrpc":"2.0","id":7,"result":{"job":"4b0c…","pending":true}}
```

The job holds the device lock until it ends. Follow it with `jobs.wait` (blocks until it ends, or `timeout_s`), `jobs.get`, or events. Cancel it with `jobs.cancel`: the work stops and the device stays as it was. A job ends `succeeded` (with `result`), `failed` (with `error`, same shape as above) or `cancelled`.

## Events

`events.subscribe {"topics": ["device.", "job."]}` (prefixes; empty = everything) makes the daemon push notifications on that connection:

```json
← {"jsonrpc":"2.0","method":"event","params":{"topic":"device.boot","data":{"device":"…","stage":"adb_online","progress":0.5}}}
```

Subscribing again replaces the filter. Topics:

| Topic | Data |
|---|---|
| `device.status` | `{device, name, status}`: status is `{state: stopped\|starting\|booting\|running\|stopping\|hibernated\|crashed, …}` |
| `device.boot` | `{device, stage: spawned\|grpc_ready\|adb_online\|booted, progress}` |
| `device.boot_failed` | `{device, error}` |
| `device.crashed` | `{device, report: {kind, reason, exit_code, log, …}}`; never restarted (D9) |
| `device.created`, `device.deleted`, `device.updated` | `{device, name}` |
| `device.hibernated` | `{device, name}`: saved and stopped by `device.hibernate` or auto-hibernate |
| `device.hook_failed` | `{device, provider, error}`: a provider's after-boot hook failed |
| `pool.leased`, `pool.released` | `{lease, device, base?}`: a pool clone was leased, or released (by `pool.release` or the reaper) |
| `job.started` | `{job, kind, device}` |
| `job.progress` | `{job, progress, message}` |
| `job.finished` | the full job |
| `job.cancelling` | `{job}` |
| `logcat` | `{device, line}` (only after `logcat.follow`) |
| `events.lagged` | `{missed}`: the connection fell behind and events were dropped |

## Connection-scoped methods

These act on the calling connection and aren't in `commands.list`:

| Method | Params | Effect |
|---|---|---|
| `hello` | `{version, client?}` | Handshake (above) |
| `commands.list` | — | Every command below, with its params schema |
| `events.subscribe` | `{topics?: string[]}` | Push matching events to this connection |
| `logcat.follow` | `{device, filter?: string[]}` | Push `logcat` events for a running device until the connection closes |
| `daemon.shutdown` | — | Stop the daemon (allowed before `hello`) |

## Commands

Providers register commands with an id, a label, a params JSON Schema and three flags: `destructive` (MCP refuses it unless allowed, D5), `longRunning` and `locksDevice`. The daemon core provides the commands below; `jidr-widgets` (sensors, phone, network, media, microphone, snapshots) and `jidr-root` (root modes) add theirs when they land. `commands.list` is always the source of truth.

<!-- commands:begin (generated from commands.list; JIDR_UPDATE_DOCS=1 cargo nextest run -p jidr protocol_doc) -->

### `app.*`

#### `app.inspect`

Package name of an APK on this machine

| Param | Type | Required | Description |
|---|---|---|---|
| `apk` | string | yes | Path of the APK on this machine. |

#### `app.launch`

Launch an installed app by package (or APK)

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `package` | string |  | Package to launch, e.g. `com.example.shop`. |
| `apk` | string |  | Or an APK on this machine whose package to launch. |

#### `app.uninstall`

Uninstall an app and its data · locks the device · **destructive**

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `package` | string | yes | Package name, e.g. `com.example.shop`. |

### `audio.*`

#### `audio.inject`

Play a WAV (or raw PCM) file as the device's microphone until it ends or audio.stop · long-running (job)

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `path` | string | yes | A WAV file on this machine: 8-, 16-, 24- or 32-bit PCM or 32/64-bit float, mono or stereo, 8–48 kHz. Or headerless 16-bit little-endian PCM with `sample_rate`. |
| `loop` | boolean |  | Start again at the end, until `audio.stop`. |
| `sample_rate` | integer |  | Headerless PCM only: its sample rate in Hz (8000–48000). |
| `channels` | integer |  | Headerless PCM only: 1 (mono, the default) or 2 (stereo). |

#### `audio.stop`

Stop the audio playing into the microphone

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

### `base.*`

#### `base.create`

Save a running device as a read-only base for pool clones (snapshot jidr-base, animations off) · long-running (job)

| Param | Type | Required | Description |
|---|---|---|---|
| `from_device` | string | yes | A running device to make the base from (root `none` or `adb`). It is stopped; the base is a copy of it. |
| `name` | string | yes | The base's name. |

#### `base.delete`

Delete a base (none of its clones may be leased) · **destructive**

| Param | Type | Required | Description |
|---|---|---|---|
| `base` | string | yes | Base name or id. |

#### `base.list`

Bases and how many clones each has leased

No params.

### `battery.*`

#### `battery.get`

Battery level, charger and health

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `battery.set`

Set battery level, charging and health

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `level` | integer |  | Charge level, 0–100 %. |
| `charging` | boolean |  | Shorthand: `true` plugs in AC, `false` unplugs. |
| `charger` | `none` \| `ac` \| `usb` \| `wireless` |  |  |
| `health` | `good` \| `failed` \| `dead` \| `overvoltage` \| `overheated` |  |  |

### `cache.*`

#### `cache.import`

Add a local file to the download cache (offline Magisk, D30)

| Param | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | A local file to add to the download cache (e.g. a Magisk APK for offline use). |
| `sha256` | string |  | Its expected SHA-256; the import fails if the file doesn't match. |

#### `cache.list`

Files in the download cache (by SHA-256)

No params.

#### `cache.prune`

Delete cached downloads (they're downloaded again when needed)

| Param | Type | Required | Description |
|---|---|---|---|
| `older_than_days` | integer |  | Only remove downloads older than this many days (default: all). |

### `camera.*`

#### `camera.get`

Camera sources used at the next start

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `camera.set`

Use the host webcam (or the emulated camera) at the next start · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `back` | CameraSource |  |  |
| `front` | CameraSource |  |  |

### `clipboard.*`

#### `clipboard.get`

The device clipboard text

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `clipboard.set`

Set the device clipboard text

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `text` | string | yes |  |

### `daemon.*`

#### `daemon.status`

Daemon version, uptime and device counts

No params.

### `dev.*`

#### `dev.get`

Developer toggles: show taps, pointer location, layout bounds, animation scale

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `dev.set`

Set developer toggles

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `show_taps` | boolean |  | Show touches (`show_touches`). |
| `pointer_location` | boolean |  | Pointer location overlay. |
| `layout_bounds` | boolean |  | Show layout bounds (`debug.layout`). |
| `animation_scale` | number |  | Window, transition and animator scales (0, 0.5, 1, 1.5, 2, 5, 10). |

### `device.*`

#### `device.clone`

Copy a stopped device under a new name; keep_snapshot makes a copy-on-write clone that resumes that snapshot · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device to copy (must be stopped). |
| `name` | string | yes | Name of the copy. |
| `keep_snapshot` | string |  | Keep this snapshot (name or id): a copy-on-write clone whose first start resumes it, in seconds (D42). Without it the copy cold-boots. |

#### `device.connect_info`

gRPC endpoint, token and adb serial of a running device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `device.create`

Create a device from a profile and a system image

| Param | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Unique device name. |
| `image` | string | yes | System image: `35`, `35:google_apis`, `35:google_apis:arm64-v8a` or `system-images;android-35;google_apis;arm64-v8a`. |
| `profile` | string |  | Hardware profile id (`profile.list`); default `phone-6.1-fhd`. |
| `ram_mb` | integer |  | RAM in MB (overrides the profile). |
| `cpus` | integer |  | Virtual CPU cores (overrides the profile). |
| `width` | integer |  | Screen width in pixels (overrides the profile). |
| `height` | integer |  | Screen height in pixels (overrides the profile). |
| `density` | integer |  | Screen density in dpi (overrides the profile). |
| `data_mb` | integer |  | Data partition size in MB (overrides the profile). |
| `root_mode` | `none` \| `adb` \| `magisk` |  | Default: the `default_root_mode` setting (`adb` unless changed), or `none` when that is `adb` and the image doesn't allow adb root (D7). |
| `agent_owned` | boolean |  | Agents may run destructive commands on it (D5). |

#### `device.delete`

Delete a device and its data · locks the device · **destructive**

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `device.details`

A device's settings, config.ini, status and disk usage

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `device.edit`

Rename a device or change its hardware (stopped) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |
| `name` | string |  | New name. |
| `ram_mb` | integer |  | RAM in MB (overrides the profile). |
| `cpus` | integer |  | Virtual CPU cores (overrides the profile). |
| `width` | integer |  | Screen width in pixels (overrides the profile). |
| `height` | integer |  | Screen height in pixels (overrides the profile). |
| `density` | integer |  | Screen density in dpi (overrides the profile). |
| `agent_owned` | boolean |  |  |
| `auto_hibernate_minutes` | integer |  | Hibernate this device after so many idle minutes (0 = never); `null` follows the `auto_hibernate_minutes` setting. |

#### `device.hibernate`

Save a running device's whole state and stop it; device.start resumes it in seconds · long-running (job) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `device.list`

All devices with their status

No params.

#### `device.logs`

Zip a device's settings, emulator and crash logs and logcat for a bug report

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `output` | string |  | Where to write the zip (default: Jidr's logs folder). |

#### `device.props.get`

A device's build props (model, brand, fingerprint…) and whether a restart is pending

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `device.props.set`

Set a Full-root device's build props; the next start (a cold boot) applies them · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id (must be in `magisk` mode). |
| `props` | BuildProps |  | The whole set: fields left out (or empty) report the stock value. |

#### `device.reset`

Factory reset: wipe all user data · locks the device · **destructive**

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `device.start`

Start a device and wait until it has booted · long-running (job) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |
| `cold_boot` | boolean |  | Ignore the quick-boot snapshot. |
| `force` | boolean |  | Start even past the host RAM budget (`resource_exhausted` otherwise). |

#### `device.stop`

Stop a device, saving its quick-boot state · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

### `file.*`

#### `file.install`

Install an APK on a running device · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `apk` | string | yes | Path of the APK on this machine. |
| `replace` | boolean |  | Replace an installed app (`-r`). |
| `grant_permissions` | boolean |  | Grant runtime permissions (`-g`). |
| `downgrade` | boolean |  | Allow a version downgrade (`-d`). |

#### `file.pull`

Copy a file from a running device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `remote` | string | yes |  |
| `local` | string | yes |  |

#### `file.push`

Copy a file to a running device (default /sdcard/Download/)

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `local` | string | yes | Local file. |
| `remote` | string |  | Device path; default `/sdcard/Download/`. |

### `fingerprint.*`

#### `fingerprint.remove`

Lift the finger off the sensor

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `fingerprint.touch`

Touch the sensor with an enrolled finger (recognised) or another (unrecognised)

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `finger` | integer |  | Enrolled finger id (1–10). `touch` only. |

### `gps.*`

#### `gps.get`

The current GPS fix

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `gps.route`

Replay a GPX or KML route at a chosen speed · long-running (job)

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `path` | string | yes | A `.gpx` or `.kml` file on this machine. |
| `speed` | number |  | Playback speed multiplier (1, 2, 4…). |
| `loop` | boolean |  | Start again from the first point at the end. |
| `mps` | number |  | Metres per second at 1× (default 10, a city drive). |

#### `gps.route_control`

Change the running route's speed, pause or loop

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `speed` | number |  |  |
| `paused` | boolean |  |  |
| `loop` | boolean |  |  |

#### `gps.route_stop`

Stop the running route replay

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `gps.set`

Set the GPS location

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `latitude` | number | yes | Decimal degrees, −90…90. |
| `longitude` | number | yes | Decimal degrees, −180…180. |
| `altitude` | number |  | Metres above sea level. |
| `bearing` | number |  | Degrees clockwise from north. |
| `speed` | number |  | Metres per second. |

### `host.*`

#### `host.budget`

Host RAM, the device budget and what each running device uses

No params.

### `image.*`

#### `image.install`

Install a system image (and the emulator if missing) · long-running (job)

| Param | Type | Required | Description |
|---|---|---|---|
| `image` | string | yes | `35`, `35:google_apis` or a full `system-images;…` package. |

#### `image.list`

System images for this host, installed or available

| Param | Type | Required | Description |
|---|---|---|---|
| `available` | boolean |  | Include images that aren't installed (needs network; falls back to installed images when offline). |

### `jobs.*`

#### `jobs.cancel`

Cancel a running job

| Param | Type | Required | Description |
|---|---|---|---|
| `job` | uuid | yes | Job id. |

#### `jobs.get`

A job's state, progress and result

| Param | Type | Required | Description |
|---|---|---|---|
| `job` | uuid | yes | Job id. |

#### `jobs.list`

Running jobs (and finished ones with all=true)

| Param | Type | Required | Description |
|---|---|---|---|
| `all` | boolean |  | Include finished jobs. |

#### `jobs.wait`

Wait for a job to finish and return it

| Param | Type | Required | Description |
|---|---|---|---|
| `job` | uuid | yes |  |
| `timeout_s` | integer |  | Give up after this many seconds and return the job as it is. |

### `key.*`

#### `key.press`

Press a hardware key (back, home, volume, D-pad, media…)

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `key` | string | yes | `back`, `home`, `recents`, `volume_up`, `volume_down`, `mute`, `power`, `menu`, `dpad_up\|down\|left\|right\|center`, `play_pause`, `next`, `previous`, `enter`, `delete`, or any W3C key name. |

### `logcat.*`

#### `logcat.clear`

Clear the device log

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `logcat.dump`

Dump the device log

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `lines` | integer |  | Only the last N lines. |
| `filter` | string[] |  | logcat filter specs, e.g. `["ActivityManager:I", "*:S"]`. |

### `media.*`

#### `media.record`

Record the screen to MP4 (H.264) until media.record_stop or max_seconds · long-running (job)

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `path` | string |  | Where to save the MP4 (default `~/Movies/Jidr/<device> <time>.mp4`). |
| `max_seconds` | integer |  | Stop on its own after this many seconds (default 3600). |
| `fps` | integer |  | Frames per second (default 30). |

#### `media.record_stop`

Stop the screen recording and save it

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `media.screenshot`

Save a PNG of the screen

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `path` | string |  | Where to save the PNG (default `~/Pictures/Jidr/<device> <time>.png`). |

### `network.*`

#### `network.get`

Wi-Fi, mobile data and speed profile

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `network.proxy.clear`

Remove the device's global HTTP proxy

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `network.proxy.get`

The device's global HTTP proxy, or null when there is none

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `network.proxy.set`

Send the device's HTTP(S) traffic through a proxy (10.0.2.2 is this Mac), e.g. Burp or mitmproxy

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `host` | string | yes | Hostname or IPv4 address as the device sees it: `10.0.2.2` is this Mac, `127.0.0.1` the device itself (with `adb reverse`). |
| `port` | integer | yes |  |

#### `network.set`

Turn Wi-Fi or mobile data on/off and set the speed/delay profile

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `wifi` | boolean |  |  |
| `mobile` | boolean |  | Mobile data. |
| `profile` | Profile |  |  |

#### `network.shaping.get`

Packet loss on the device's networks (needs Shell or Full root; through the on-device helper)

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `network.shaping.set`

Drop a percentage of the device's outgoing packets on Wi-Fi and mobile data (netem; needs Shell or Full root; a cold boot clears it) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id (Shell root or Full root). |
| `loss_pct` | number | yes | Percent of outgoing packets to drop, 0–100 (decimals allowed); 0 removes the loss. |

### `phone.*`

#### `phone.call`

Simulate an incoming call or change its state

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `number` | string | yes | Phone number, digits with optional leading +. |
| `action` | CallAction |  |  |

#### `phone.gsm`

Mobile signal bars (0–4) and data/voice registration

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `signal` | integer |  | Mobile signal bars, 0 (none) to 4 (great). |
| `data` | `unregistered` \| `home` \| `roaming` \| `searching` \| `denied` \| `off` \| `on` |  |  |
| `voice` | `unregistered` \| `home` \| `roaming` \| `searching` \| `denied` \| `off` \| `on` |  |  |

#### `phone.sms`

Deliver an SMS to the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `from` | string | yes | Sender number. |
| `text` | string | yes |  |

### `pool.*`

#### `pool.get`

Lease a throwaway clone of a base, booted from its snapshot; returns its serial. Release it when done · long-running (job)

| Param | Type | Required | Description |
|---|---|---|---|
| `base` | string | yes | Base name or id. |
| `ttl_s` | integer |  | Lease lifetime in seconds. Without it the lease lasts as long as this client connection, plus 2 minutes. |
| `timeout_s` | integer |  | How long to wait for room in the RAM budget (default 120 s). |

#### `pool.list`

Current leases

No params.

#### `pool.release`

Give a leased clone back: it is stopped and deleted

| Param | Type | Required | Description |
|---|---|---|---|
| `lease` | uuid |  | The lease id from `pool.get`. |
| `device` | string |  | Or the pool device's name or id. |

#### `pool.renew`

Extend a lease (TTL leases, or one in its grace period)

| Param | Type | Required | Description |
|---|---|---|---|
| `lease` | uuid | yes |  |
| `ttl_s` | integer |  | New lifetime from now, in seconds (default 2 h). |

### `profile.*`

#### `profile.list`

Built-in hardware profiles

No params.

### `root.*`

#### `root.auto_grant`

Grant every app root without a prompt on a Full-root device (D22, off by default) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id (must be in `magisk` mode). |
| `enabled` | boolean | yes | Grant every app root without a prompt (D22, off by default). |

#### `root.cacert.install`

Trust a CA certificate (PEM or DER, e.g. Burp's) as a system cert on a running Full-root device; restarts it · long-running (job) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id (must be in `magisk` mode and running). |
| `path` | string | yes | A CA certificate, PEM or DER (`.pem`, `.crt`, `.cer`, `.der`), e.g. Burp's or mitmproxy's. |

#### `root.cacert.list`

The CA certificates Jidr added to a running Full-root device and whether each is trusted

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `root.cacert.remove`

Remove a CA certificate Jidr added (or all of them); restarts the device · long-running (job) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id (must be in `magisk` mode and running). |
| `cert` | string |  | The certificate: its store file name (`a4ac454c.0`), subject hash or SHA-256 (or a unique prefix of 8+ hex digits). Leave out with `all`. |
| `all` | boolean |  | Remove every certificate Jidr installed (and the module). |

#### `root.cacert.verify`

Check that every CA certificate Jidr added is in the system store apps read · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `root.helper.status`

Start the rooted on-device helper if needed (Shell or Full root) and report its version, uid and SELinux context

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `root.module.install`

Install a Magisk module zip and reboot (Full-root devices only) · long-running (job) · locks the device · **destructive**

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id (must be in `magisk` mode). |
| `path` | string | yes | A local Magisk module zip. |

#### `root.set`

Set a device's root mode (applied live if it is running; the next start is a cold boot) · long-running (job) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |
| `mode` | `none` \| `adb` \| `magisk` | yes | `none`, `adb` (images that allow it) or `magisk`. |
| `magisk_sha256` | string |  | SHA-256 of a Magisk APK already in the download cache (the CLI imports it with `--magisk <path>` before calling this); overrides the pinned release (D30, offline). |

#### `root.status`

Show a device's root mode and health

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `root.verify`

Check a running device's root against its mode · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `root.writable_system`

Make /system writable on a shell-root (adb) device (-writable-system + adb remount), or back to read-only; cold boots it if running · long-running (job) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id (must be in `adb` root mode, on API 31 or later, to enable). |
| `enabled` | boolean | yes | `true`: boot with a writable `/system` (`-writable-system` + `adb remount`). `false`: back to the read-only stock system image. |

### `sdk.*`

#### `sdk.accept_license`

Record consent to a license text by its SHA-256 (D8)

| Param | Type | Required | Description |
|---|---|---|---|
| `id` | string | yes | License id, e.g. `android-sdk-license`. |
| `sha256` | string | yes | SHA-256 of the license text you were shown (from `sdk.licenses`). |

#### `sdk.install`

Install SDK packages into Jidr's private SDK · long-running (job)

| Param | Type | Required | Description |
|---|---|---|---|
| `packages` | string[] | yes | SDK package paths, e.g. `emulator`, `system-images;android-35;google_apis;arm64-v8a`. |

#### `sdk.licenses`

Licenses still to accept before installing packages, with their text

| Param | Type | Required | Description |
|---|---|---|---|
| `packages` | string[] | yes | SDK package paths, e.g. `emulator`, `system-images;android-35;google_apis;arm64-v8a`. |

#### `sdk.status`

Where the SDKs are, emulator version, hypervisor, free disk

No params.

### `sensor.*`

#### `sensor.get`

Read a sensor value

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `sensor` | `acceleration` \| `gyroscope` \| `magnetic_field` \| `temperature` \| `proximity` \| `light` \| `pressure` \| `humidity` | yes |  |

#### `sensor.rotate`

Rotate the device (0, 90, 180, 270°)

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `angle` | integer | yes | Device rotation in degrees: 0, 90, 180 or 270 (counter-clockwise). |

#### `sensor.set`

Set a motion or environment sensor value

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `sensor` | `acceleration` \| `gyroscope` \| `magnetic_field` \| `temperature` \| `proximity` \| `light` \| `pressure` \| `humidity` | yes |  |
| `values` | number[] | yes | Values in the sensor's units (3 for vectors, 1 for scalars). |

### `settings.*`

#### `settings.get`

Daemon-wide settings: default root mode, quick boot on stop, RAM budget, auto-hibernate

No params.

#### `settings.set`

Change daemon-wide settings (only the fields given)

| Param | Type | Required | Description |
|---|---|---|---|
| `default_root_mode` | `none` \| `adb` \| `magisk` |  | Root mode for `device.create` without `root_mode`. |
| `quick_boot` | boolean |  | Save a quick-boot snapshot on stop (D29); from each device's next start. |
| `budget_percent` | integer |  | Share of host RAM running devices may use, 10–100 percent (D42). |
| `auto_hibernate_minutes` | integer |  | Hibernate running devices idle this many minutes; 0 = off. |

### `snapshot.*`

#### `snapshot.delete`

Delete a snapshot · locks the device · **destructive**

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `name` | string | yes | Snapshot name. |

#### `snapshot.list`

Snapshots with their root-mode tags, sizes and thumbnails

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device name or id. |

#### `snapshot.load`

Restore a snapshot (refused when it was taken under another root mode) · long-running (job) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `name` | string | yes | Snapshot name. |

#### `snapshot.save`

Save a snapshot of a running device · long-running (job) · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `name` | string | yes | Snapshot name. |
<!-- commands:end -->

## Changelog

| Version | Change |
|---|---|
| 0.1 | Initial protocol: hello, errors, jobs, events, core device/sdk/image/profile/file/logcat/jobs commands. |
| 0.1 (additive) | Phase 5b: `cache.list`, `cache.import`, `cache.prune`, `device.logs`. No existing command changed. |
| 0.1 (additive) | G15: `root.auto_grant` (D22 "auto-grant all", from the root provider). No existing command changed. |
| 0.1 (additive) | B4: `settings.get` / `settings.set {default_root_mode?, quick_boot?}` (daemon-wide, `settings.json`). `device.create` without `root_mode` uses `default_root_mode` (still `adb` by default, `none` where adb root isn't allowed); `quick_boot: false` launches with `-no-snapshot-save` from a device's next start. |
| 0.1 (additive) | B5: `Job.bytes {done, total}` and a `bytes` field on `job.progress` while a job downloads (`image.install`, `sdk.install`); absent otherwise. G14: `image.list` fills `pristine_ramdisk_sha256` for installed images, and `device.details` gains `pristine_ramdisk_sha256`. G16: the command reference now lists every provider's commands (widgets, root), generated by `cargo nextest run -p jidr protocol_doc`. No existing field changed meaning. |
| 0.1 (additive) | Phase 9f: `device.props.get`, `device.props.set` (a Full-root device's build props, from the root provider). No existing command changed. |
| 0.1 (additive) | `network.proxy.get`, `network.proxy.set {host, port}`, `network.proxy.clear` (the device's global HTTP proxy, from the widgets provider). No existing command changed. |
| 0.1 (additive) | `root.cacert.install {path}`, `root.cacert.list`, `root.cacert.verify`, `root.cacert.remove {cert?, all?}` (CA certificates in a Full-root device's system store, from the root provider). No existing command changed. |
| 0.1 (additive) | Phase 13a (D42): new error code `resource_exhausted` (-32009, CLI exit 9) from `device.start` past the host RAM budget; `device.start` gains `force`. New `host.budget` → `{total_mb, available_mb, budget_percent, budget_mb, used_mb, devices: [{id, name, expected_mb, measured_mb?}]}`. `settings` gains `budget_percent` (default 75, 10–100). `Device` gains `footprint_mb` (peak of its last run); `HardwareProfile` gains `audio`, `camera_back`, `camera_front`; `LaunchPrefs` gains `audio`. New profile `agent-lean`. No existing field changed meaning. |
| 0.1 (additive) | Phase 13b (D42): `device.clone` gains `keep_snapshot` (a snapshot name or id with a non-empty `ram.bin`): a copy-on-write clone that keeps only that snapshot (and its root-mode tag), drops `default_boot` and the locks, and rewrites the AVD name and folder in its ini files. `LaunchPrefs` gains `resume_snapshot`: the next start launches with `-snapshot <id>` (and no exit save into it), then it is cleared. Without `keep_snapshot`, `device.clone` is unchanged. |
| 0.1 (additive) | Phase 13c (D42): new device state `hibernated` (not active; uses no RAM) and event `device.hibernated`. New `device.hibernate` (long-running, locks the device): a named snapshot `jidr-hibernate` via SnapshotService, refused with `sdk` (the device keeps running) when its `ram.bin` is empty, then an exit without the quick-boot save; `device.start` resumes it (`-snapshot jidr-hibernate`) and the snapshot's root tag is kept (D29); `device.stop` leaves it hibernated, `device.reset` discards it. `settings` gains `auto_hibernate_minutes` (0 = off, the default); `Device` and `device.edit` gain `auto_hibernate_minutes` (`0` = never, `null` = the setting). `resource_exhausted` details gain `hibernate_candidates: [{id, name, idle_s}]`, the longest-idle running devices first. |
| 0.1 (additive) | Phase 13d (D42): new commands from the pool provider: `base.create {from_device, name}` (long-running), `base.list`, `base.delete {base}` (destructive), `pool.get {base, ttl_s?, timeout_s?}` (long-running) → `{lease, device, serial, connect}`, `pool.release {lease \| device}`, `pool.renew {lease, ttl_s?}`, `pool.list`. A lease without `ttl_s` is tied to the calling connection and expires 2 minutes after it closes; the daemon reaps expired leases every 30 s and stays up while any exist (D4). `Device` gains `base` (read-only; `device.start` refuses it) and `pool_base`. Events `pool.leased`, `pool.released`. A full budget makes `pool.get` wait, then fail with `resource_exhausted`. |
| 0.1 (fix) | `root.set` (mode `magisk`) and `root.verify`: `magisk_version` is now the version alone (`30.7`), not the raw `magisk -v` output (`30.7:MAGISK:R`). The field and its type are unchanged. |
| 0.1 (additive) | #25: `root.writable_system {device, enabled}` (long-running, locks the device) → `{device, writable_system, changed, applied: "unchanged" \| "next_boot" \| "live", system_writable?}`: a writable `/system` on an `adb`-root device (`-writable-system` launch option, then `adb remount` from the boot hook), stored in the existing `RootState.writable_system`. Enabling is refused with `usage` unless the device is in `adb` mode on an image that allows adb root, at API 31 or later; changing it forces a cold boot (D29). `root.set` to `none` or `magisk` resets `writable_system` to `false`, and `base.create` refuses a source with it on (`usage`). No existing field changed meaning. |
| 0.1 (additive) | #70: `audio.inject {device, path, loop?, sample_rate?, channels?}` (long-running) plays a WAV (8/16/24/32-bit PCM or float, mono or stereo, 8–48 kHz; 24/32-bit and float are converted to 16-bit) or headerless 16-bit PCM as the device's microphone over gRPC `injectAudio`, and ends with `{path, duration_s, played_s, loops, stopped, sample_rate, channels, format, converted_from}`; `audio.stop {device}` → `{stopped}` ends it. Unsupported files are `usage`; a microphone another client already feeds is `busy`. From the widgets provider; no existing command changed. |
| 0.1 (additive) | #26 (ADR-0003, Proposed): `root.helper.status {device}` → `{device, name, root_mode, helper: {version, protocol, sha256, pid, uid, selinux?, arch}, port}` starts the rooted on-device helper if needed (root provider). `network.shaping.get {device}` and `network.shaping.set {device, loss_pct}` (locks the device) → `{loss_pct, interfaces: [{name, qdisc?, loss_pct}]}` drop a percentage of outgoing packets through the helper (`tc … netem`, widgets provider; `loss_pct` is null when interfaces differ). They need root mode `adb` or `magisk` (`usage` otherwise, and when the kernel lacks netem, with `error.data.helper_error`); a helper that can't start as root is `root_failed`. No existing command changed. |
