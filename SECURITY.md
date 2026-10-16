# Security policy

## Supported versions

Jidr is pre-1.0. Security fixes go into the latest release only; please update before reporting.

## Reporting a vulnerability

Please **don't open a public issue** for a security problem. Report it privately through GitHub: on the repository's **Security** tab, choose **Report a vulnerability**. Include what you found, how to reproduce it, the Jidr version, your macOS version, and the impact you expect.

You can expect an acknowledgement within 7 days and an assessment within 30 days. We'll keep you updated, agree on a disclosure date with you, and credit you in the release notes unless you'd rather not be named.

## What's in scope

Jidr runs a local daemon and changes Android emulator images, so these areas matter most:

- **The daemon's socket:** another local user, or an unprivileged process, controlling devices through the Unix socket.
- **The MCP server:** an agent getting past the destructive-action policy (D5), for example deleting or resetting a device that isn't `agent-owned` without `--allow-destructive`.
- **Downloads:** the SDK, system images or Magisk installed without checking their SHA-256 (Magisk is pinned in `assets/magisk-pins.toml`), or fetched over a path an attacker can tamper with.
- **Root and ramdisk patching:** a patched ramdisk affecting another device or the original image, or root reaching beyond the emulated device onto the Mac.
- **File handling:** APKs, zips, GPX/KML files or snapshots dropped on the app writing outside the places they belong.
- **The release artifact:** anything that undermines the DMG's signature or notarization.

Out of scope: the security of the Android images themselves (report those to Google), Magisk (report to its project), and the fact that a rooted emulator has root inside the emulator. That's the feature.

## Telemetry

Jidr sends no telemetry and has no accounts (D17). It contacts Google's SDK servers, GitHub (only to download the pinned Magisk release) and OpenStreetMap's tile servers for the GPS map, and nothing else.
