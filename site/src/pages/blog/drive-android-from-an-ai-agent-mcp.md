---
layout: ../../layouts/Post.astro
title: "Driving an Android emulator from an AI agent over MCP"
description: "Give Claude or any MCP client a real Android device on your Mac: create and boot it, install an app, look at the screen, tap, type and read the logs."
date: 2026-10-17
---

An AI coding agent can write an Android feature, but it usually can't see it run. The Model Context Protocol (MCP) closes that loop: an MCP server exposes tools, and the agent calls them. Give it tools for a real Android device, and it can install the build it just made, look at the screen, tap through the flow and read the crash in logcat.

[Jidr](https://github.com/AbdurahmanAlmehdi/jidr) ships one: `jidr mcp`, an MCP server over stdio for Android emulators on Apple Silicon Macs.

## Setup

Install Jidr (`brew install --cask abdurahmanalmehdi/tap/jidr`, or the DMG from the [release](https://github.com/AbdurahmanAlmehdi/jidr/releases/latest)), then add it to your agent.

Claude Code:

```bash
claude mcp add jidr -- jidr mcp
```

Claude Desktop: open `jidr-mcp-<version>-darwin-arm64.mcpb` from the release, or add this to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "jidr": { "command": "/opt/homebrew/bin/jidr", "args": ["mcp"] }
  }
}
```

Cursor and other clients take the same `command` and `args`. The server starts the Jidr daemon if it isn't running, so there's nothing else to launch.

## The tools

Two tools reach everything Jidr can do: `command_list` returns every command with its JSON Schema, and `command_run` runs one. Long operations, such as booting a device or downloading an image, are waited for up to a timeout and come back as a job otherwise.

The other nine are fast paths for the loop an agent repeats most:

| Tool | What it does |
|---|---|
| `screenshot` | PNG of the screen, fitted in 1024×1024, with the scale |
| `tap`, `swipe` | Touch input, in screenshot coordinates by default |
| `type_text`, `press_key` | Text into the focused field; Home, Back, Enter and other keys |
| `ui_dump` | The uiautomator tree: text, resource ids and bounds |
| `logcat_tail` | The last lines of the log, optionally filtered |
| `jobs_list`, `jobs_cancel` | Long-running operations |

Every tool carries MCP's read-only, destructive and idempotent hints, and errors come back as structured results (`not_found`, `busy`, `boot_timeout`, …) that an agent can act on.

## A session

Asked to "check the sign-up screen of my debug build on Android 15", an agent's calls look like this:

```json
{"tool": "command_run", "arguments": {"command": "device.create", "params": {"name": "agent-phone", "image": "35"}}}
{"tool": "command_run", "arguments": {"command": "device.start", "params": {"device": "agent-phone"}}}
{"tool": "command_run", "arguments": {"command": "file.install", "params": {"device": "agent-phone", "apk": "app/build/outputs/apk/debug/app-debug.apk"}}}
{"tool": "command_run", "arguments": {"command": "app.launch", "params": {"device": "agent-phone", "apk": "app/build/outputs/apk/debug/app-debug.apk"}}}
{"tool": "screenshot", "arguments": {"device": "agent-phone"}}
{"tool": "ui_dump", "arguments": {"device": "agent-phone"}}
{"tool": "tap", "arguments": {"device": "agent-phone", "x": 512, "y": 1730, "coords": "device"}}
{"tool": "type_text", "arguments": {"device": "agent-phone", "text": "test@example.com", "enter": true}}
{"tool": "logcat_tail", "arguments": {"device": "agent-phone", "lines": 200, "filter": "AndroidRuntime"}}
```

Two habits make agents much more reliable here:

- **Find targets in `ui_dump`, not in pixels.** The tree gives each view's bounds in device pixels, so the agent can tap the centre of the "Sign up" button with `coords: "device"`. Guessing from a scaled screenshot misses small targets.
- **Check logcat after every step that can crash.** A stack trace under `AndroidRuntime` explains a blank screen faster than another screenshot.

On an M1 Pro, a screenshot takes about 0.4–1.1 s, a tap 0.2–0.4 s and typing 0.7–1.2 s. `ui_dump` is the slow one, at 4–6 s, because uiautomator is. A cold boot takes 25–32 s.

## Safety

An agent with a device can do damage, so destructive commands are refused by default: deleting a device, factory reset, uninstalling an app. They're allowed only on devices the agent created itself (marked agent-owned, as `agent-phone` above) or when you start the server with `jidr mcp --allow-destructive`. A refusal says why, so the agent can ask you instead of retrying.

Root is a device setting like any other. If the task needs it, the agent can call `root.set` with `magisk` and verify it with `root.verify`; the [rooting post](/blog/root-android-emulator-apple-silicon/) explains what that does.

## In CI

The same commands work without an agent: every MCP call has a `jidr` CLI equivalent with `--json` output and stable exit codes, so a flow an agent worked out can become a script on a macOS runner.

## Further reading

- [MCP tools in the Jidr docs](https://github.com/AbdurahmanAlmehdi/jidr/blob/main/docs/mcp.md)
- [The Model Context Protocol](https://modelcontextprotocol.io)
- [Jidr's CLI reference](https://github.com/AbdurahmanAlmehdi/jidr/blob/main/docs/cli.md)
