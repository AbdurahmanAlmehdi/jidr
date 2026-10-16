# Jidr control protocol (v0.1)

The daemon (`jidr daemon`) owns every device. The GUI, the CLI (`jidr`) and the MCP server (`jidr mcp`) are clients of this protocol and nothing else, so anything one client can do, all can. Reference client: `crates/jidr-client`.

Frozen as **v0.1** at the git tag `protocol-v0.1`. A change after the tag needs an entry in the [changelog](#changelog).

## Transport

- **Socket:** a Unix domain socket at `~/Library/Application Support/Jidr/daemon.sock` (`$JIDR_HOME/data/daemon.sock` when `JIDR_HOME` is set), mode `0600`. One daemon per user; a second one refuses to start.
- **Framing:** [JSON-RPC 2.0](https://www.jsonrpc.org/specification), one JSON object per line (newline-delimited JSON, as in MCP's stdio transport). Lines are at most 4 MB.
- **Concurrency:** requests on one connection run concurrently; match answers by `id`. Notifications from the daemon have no `id`.
- **Lifetime (D4):** clients start the daemon on demand (`jidr daemon`, detached). It exits after 10 minutes with no running device, no connected client and no active job. Emulators that outlive a daemon are re-adopted by the next one.

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

Protocol errors use the standard JSON-RPC codes. Jidr errors carry one of eight codes, which are also the CLI's exit codes, in `error.data.code`; `error.data.details` holds structured extras.

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
| `device.status` | `{device, name, status}`: status is `{state: stopped\|starting\|booting\|running\|stopping\|crashed, …}` |
| `device.boot` | `{device, stage: spawned\|grpc_ready\|adb_online\|booted, progress}` |
| `device.boot_failed` | `{device, error}` |
| `device.crashed` | `{device, report: {kind, reason, exit_code, log, …}}`; never restarted (D9) |
| `device.created`, `device.deleted`, `device.updated` | `{device, name}` |
| `device.hook_failed` | `{device, provider, error}`: a provider's after-boot hook failed |
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

Providers register commands with an id, a label, a params JSON Schema and three flags: `destructive` (MCP refuses it unless allowed, D5), `longRunning` and `locksDevice`. The daemon core provides the commands below; `jidr-widgets` (sensors, phone, network, media, snapshots) and `jidr-root` (root modes) add theirs when they land. `commands.list` is always the source of truth.

<!-- commands:begin (generated from commands.list; JIDR_UPDATE_DOCS=1 cargo nextest run -p jidr-daemon protocol_doc) -->

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

### `daemon.*`

#### `daemon.status`

Daemon version, uptime and device counts

No params.

### `device.*`

#### `device.clone`

Copy a stopped device under a new name · locks the device

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes | Device to copy (must be stopped). |
| `name` | string | yes | Name of the copy. |

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
| `root_mode` | `none` \| `adb` \| `magisk` |  | Default: `adb` where the image allows it, else `none` (D7). |
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

#### `device.list`

All devices with their status

No params.

#### `device.logs`

Zip a device's settings, emulator and crash logs and logcat for a bug report

| Param | Type | Required | Description |
|---|---|---|---|
| `device` | string | yes |  |
| `output` | string |  | Where to write the zip (default: Jidr's logs folder). |

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

### `profile.*`

#### `profile.list`

Built-in hardware profiles

No params.

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
<!-- commands:end -->

## Changelog

| Version | Change |
|---|---|
| 0.1 | Initial protocol: hello, errors, jobs, events, core device/sdk/image/profile/file/logcat/jobs commands. |
| 0.1 (additive) | Phase 5b: `cache.list`, `cache.import`, `cache.prune`, `device.logs`. No existing command changed. |
| 0.1 (additive) | G15: `root.auto_grant` (D22 "auto-grant all", from the root provider). No existing command changed. |
| 0.1 (additive) | Phase 9f: `device.props.get`, `device.props.set` (a Full-root device's build props, from the root provider). No existing command changed. |
