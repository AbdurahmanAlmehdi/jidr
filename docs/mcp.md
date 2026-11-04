# Jidr for AI agents (MCP)

`jidr mcp` serves the [Model Context Protocol](https://modelcontextprotocol.io) on stdio. Agents get every Jidr command plus fast tools for driving a device: look, tap, type, read the UI. It starts the Jidr daemon if it isn't running.

## Add Jidr to Claude

Claude Code:

```bash
claude mcp add jidr -- jidr mcp
```

Claude Desktop: download `jidr-mcp-<version>-darwin-arm64.mcpb` from the [latest release](https://github.com/AbdurahmanAlmehdi/jidr/releases/latest) and open it, or add Jidr by hand to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "jidr": { "command": "/opt/homebrew/bin/jidr", "args": ["mcp"] }
  }
}
```

Cursor (`~/.cursor/mcp.json`) takes the same `mcpServers` entry. `jidr uninstall` removes these entries again (D31).

Each release also publishes Jidr to the [MCP Registry](https://registry.modelcontextprotocol.io) as `io.github.AbdurahmanAlmehdi/jidr`, pointing at that `.mcpb`.

## Tools

| Tool | Hints | What it does |
|---|---|---|
| `command_list {prefix?}` | read-only | Every Jidr command with its params schema and flags |
| `command_run {command, params, timeout_s?}` | may be destructive | Runs any command; long-running ones (boot, image install) are waited for, up to `timeout_s` (900 s), then returned as a job |
| `pool_get {base, ttl_s?, timeout_s?}` | interacts | A throwaway device of your own: a copy-on-write clone of a base, resumed from its snapshot in seconds. Returns `{lease, device, serial, connect}`. Without `ttl_s` the lease lasts as long as this MCP session, plus 2 minutes |
| `pool_release {lease \| device}` | destructive (your clone only) | Gives the clone back: it is stopped and deleted |
| `jobs_list {all?}`, `jobs_cancel {job}` | read-only / idempotent | Long-running operations |
| `screenshot {device}` | read-only | PNG of the screen fitted in 1024×1024, plus the screen size and scale |
| `tap {device, x, y, coords?}` | interacts | Tap; coordinates are in the screenshot image unless `coords: "device"` |
| `swipe {device, x1, y1, x2, y2, duration_ms?, coords?}` | interacts | Swipe or drag (scroll, open the app drawer) |
| `type_text {device, text, enter?}` | interacts | Type printable ASCII into the focused field |
| `press_key {device, key}` | interacts | `GoHome`, `GoBack`, `AppSwitch`, `Enter`, `Power`, volume, arrows… |
| `ui_dump {device}` | read-only | uiautomator XML: text, ids and bounds to find what to tap |
| `logcat_tail {device, lines?, filter?}` | read-only | Recent log lines (the last N lines that match the filter) |
| `ca_cert_install {device, path}` | may restart the device | Trust a CA certificate (PEM or DER, on this Mac) as a system cert on a running Full-root device, then verify; `root.cacert.list` / `.verify` / `.remove` go through `command_run` |
| `command_run root.writable_system {device, enabled}` | restarts a running device | A writable `/system` on an `adb`-root device, or back to read-only (docs/root.md); refused with `usage` on other root modes and Play Store images |
| `proxy_set {device, host?, port}`, `proxy_clear {device}` | idempotent | The device's global HTTP proxy, for Burp or mitmproxy on the Mac (`host` defaults to `10.0.2.2`, the Mac as the emulator sees it) |

Every tool carries `readOnlyHint`, `destructiveHint`, `idempotentHint` and `openWorldHint` annotations. Jidr errors come back as tool results with `isError: true` and structured `{code, message, data}` (codes as in docs/control-protocol.md), so an agent can recover, e.g. start a stopped device after `not_found`.

## Devices for agents (D42)

Use a device of your own, never the user's (for example not `emulator-5554`):

- **Throwaway:** `pool_get {base}` (bases: `command_run base.list`; a person makes one with `jidr base create <device> <name>`). Use the returned `serial` with adb, Flutter or Gradle, and call `pool_release` when done. If the session ends without it, the daemon deletes the clone 2 minutes later. For long tasks pass `ttl_s` and extend it with `command_run pool.renew`.
- **Persistent:** `command_run device.create {name, image, profile: "agent-lean"}` (about 4 GB of host RAM instead of ~7.6 GB) and `device.start`; hibernate it with `device.hibernate` when idle.
- **Full machine:** `pool_get` waits up to `timeout_s` (120 s) for room in the host RAM budget, then fails with `resource_exhausted`. `device.start` fails with it at once; its `data.devices` says what uses the RAM and `data.hibernate_candidates` which running devices are idle longest. Hibernate or release your own devices, or wait; don't hibernate the user's.

## Safety policy (D5)

By default an agent can read, interact, start and stop. Commands flagged destructive (`device.delete`, `device.reset`, …) are refused unless:

- the device is **agent-owned**: devices an agent creates through `command_run device.create` are agent-owned automatically, and a person can mark one with `jidr device edit <name> --agent-owned true`; or
- the server runs as `jidr mcp --allow-destructive`.

Refusals carry `data.policy = "D5"`. Connection-level protocol methods (`hello`, `events.subscribe`, `logcat.follow`, `daemon.shutdown`) aren't callable through MCP.

## Measured latency (not gated)

From the five scripted agent tasks (`cargo nextest run -p jidr-mcp --features emulator`, M1 Pro, API 31 arm64): screenshot ~0.4–1.1 s, tap ~0.2–0.4 s, type_text ~0.7–1.2 s, press_key ~0.01–0.1 s, swipe 1–4 s, logcat_tail ~0.2–0.5 s, ui_dump 4–6 s median (uiautomator itself is slow), cold boot via `command_run device.start` 25–32 s.
