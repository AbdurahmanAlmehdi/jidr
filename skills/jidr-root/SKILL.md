---
name: jidr-root
description: "Root a Jidr Android emulator and work with root: choose a root mode (none, adb, magisk), switch with `jidr root set` and check uid 0 with `jidr root verify`, install Magisk modules, handle Magisk's su prompt and the opt-in auto-grant, revert to stock, trust a proxy's CA as a system certificate (`jidr cert`) route traffic through Burp or mitmproxy (`jidr proxy`), and drop a share of the device's packets (`jidr network loss`). Use when the user asks for a rooted emulator, su, Magisk, a Magisk module, intercepting an app's HTTPS traffic, testing an app on a lossy network, or asks why Play Integrity fails. Load jidr for the basics."
---

# Root on a Jidr device

Root is a property of each Jidr device (`root_mode`), never a change to the shared system image. Use the jidr skill to find or create a device first. Root only devices you created or the user pointed you to: a root change cold-boots the device and drops what is on its screen.

## Choose a mode

| Mode | What you get | Images | Check |
|---|---|---|---|
| `none` | stock Android | all | `adb shell id` → `uid=2000(shell)` |
| `adb` | a root adb shell (`adb root`), no `su` for apps | `default`, `google_apis`, `aosp_atd` (not `google_apis_playstore`) | `adb shell id` → `uid=0(root)` |
| `magisk` | full root: `su`, Magisk modules, system CA certificates, build props | all (Play Store images are beta) | `adb shell su -c id` → `uid=0(root)` |

New devices get `adb` where the image allows it, `none` elsewhere (D7). Pick the lowest mode the task needs:

- Reading app data, `pm`, `setprop`, pulling `/data`: `adb` is enough.
- An app that calls `su`, a Magisk module, intercepting HTTPS, changing build props: `magisk`.

```bash
jidr device list                       # the ROOT column shows each device's mode
jidr root status Phone                 # mode, health (unverified, ok, failed), Magisk version
jidr -q device create Rooted --image 35 --profile phone-6.1-fhd --root magisk --agent-owned
```

## Switch and verify

```bash
jidr root set Phone magisk             # one job: fetch Magisk, patch this device's ramdisk, cold boot, set up, verify
jidr root verify Phone                 # runs `su -c id` (or `id` for adb) and checks uid 0 against the mode
jidr root set Phone none               # back to stock
```

- A root change always forces a **cold boot** (D29): quick-boot state is not reused, so anything open on the device is gone. On a running device it is applied right away; on a stopped device, at the next start.
- Snapshots are tagged with the root mode they were taken in. Take new ones after switching.
- `root set … magisk` downloads the pinned Magisk release from GitHub and checks its SHA-256. Offline, or if the user has the APK, pass it: `jidr root set Phone magisk --magisk /path/to/Magisk-v30.7.apk` (still checked).
- `adb` on a Play Store image is refused with exit 2: use `magisk` instead.
- Exit **7** means root failed. Read `jidr root status Phone`, then retry `root set` or revert with `jidr root set Phone none`. A failed Magisk setup is always revertable.

Check it yourself too, with the serial from `jidr device list`:

```bash
adb -s emulator-5556 shell su -c id          # uid=0(root) … on magisk
adb -s emulator-5556 shell su -c 'magisk -v' # the Magisk version
```

### Play Store images (beta)

`google_apis_playstore` images refuse `adb root`, so the shell's `su` can't be pre-approved (ADR-0002). During `root set … magisk`, Magisk shows a prompt on the device: **the user** has to tap Grant within 60 seconds. Tell them before you run it, and tell them to open the device in the Jidr app (`open -a Jidr`). If nobody taps, the job fails with `awaiting_user_grant`: ask the user, then run `jidr root verify Phone` again or revert. Never try to get around the prompt.

## Who gets su

On a `magisk` device:

- The adb shell is granted `su` automatically (on Play Store images, after the user's one tap).
- Every app gets Magisk's normal allow/deny prompt on the device the first time it asks (D22). An app waiting on that prompt looks hung: take a screenshot and ask the user to tap, or tap Grant yourself only if the user asked you to grant that app.
- "Auto-grant all" gives every app root without a prompt. It is **off by default**. Turn it on only when the user explicitly asks for it, for that device, and turn it off again when the task is done:

```bash
jidr call root.auto_grant '{"device":"Phone","enabled":true}'    # only on the user's explicit request
jidr call root.auto_grant '{"device":"Phone","enabled":false}'   # back to the prompt
```

Leaving `magisk` mode resets auto-grant to off.

## Magisk modules

On a running `magisk` device, install a module zip from this machine; the device reboots to load it (D35):

```bash
jidr call root.module.install '{"device":"Phone","path":"/Users/me/Downloads/module.zip"}'
```

Use an absolute path. Other modes refuse it (exit 2). Install only modules the user gave you or named: a module runs as root at every boot. It is flagged destructive, so through MCP it needs an agent-owned device or `--allow-destructive`.

## Intercept HTTPS: CA certificate and proxy

Apps that target Android 7+ ignore user CAs, so the proxy's CA must go into the **system** store. That needs a running `magisk` device:

```bash
jidr root set Phone magisk                              # once
jidr cert install Phone /Users/me/.mitmproxy/mitmproxy-ca-cert.pem  # PEM or DER; restarts the device, then verifies
jidr cert list Phone                                    # Jidr's CAs and whether each is trusted
jidr cert verify Phone                                  # exit 7 unless every one is in the store apps read
jidr proxy set Phone 8080                               # 10.0.2.2:8080, this Mac's port 8080
jidr proxy show Phone
```

- Burp exports its CA as DER (Proxy › Proxy settings › Import / export CA certificate); mitmproxy keeps it in `~/.mitmproxy/mitmproxy-ca-cert.pem`.
- In the emulator, `10.0.2.2` is the Mac's `127.0.0.1`. `127.0.0.1` on the device is the device itself.
- The proxy works on any root mode and takes effect at once; only the certificate needs `magisk`.
- An app that pins its certificates still refuses the proxy. Say so rather than trying to patch the app.

Undo it when you're done, so the device's traffic goes direct again:

```bash
jidr proxy clear Phone
jidr cert remove Phone a4ac454c.0       # a store file name, subject hash or SHA-256; restarts the device
jidr cert remove Phone --all            # every CA Jidr added
```

With MCP: `ca_cert_install {device, path}`, `proxy_set {device, port, host?}` and `proxy_clear {device}`; other `root.cacert.*` commands go through `command_run`.

## Packet loss (on-device helper)

On a running `adb` or `magisk` device, Jidr can drop a share of the device's outgoing packets, to test an app on a bad network:

```bash
jidr network loss Phone 10              # drop 10% on Wi-Fi and mobile data
jidr network loss Phone                 # show it per interface
jidr network loss Phone 0               # remove it
jidr root helper Phone                  # the rooted helper that does it: version, uid 0, SELinux context
```

- It runs through Jidr's own on-device helper, which the daemon starts as root when needed. A `none` device refuses it (exit 2); set `adb` first.
- A cold boot clears the loss; remove it when you're done anyway.
- Through MCP: `command_run` with `network.shaping.set {device, loss_pct}` and `network.shaping.get {device}`.

## Revert

```bash
jidr root set Phone none                # or adb; deletes the patched ramdisk and cold-boots stock
jidr root status Phone
```

Reverting stops Magisk, so its modules, system certificates, auto-grant and changed build props stop applying. The app data stays.

## Play Integrity

Play Integrity cannot pass on any emulator, rooted or not, and Zygisk or DenyList only hide root from local checks. Tell the user this when an app (banking, games, Google Wallet) refuses to run; don't try spoofing props or modules to make it pass.

## Don't

- Don't change the root mode of a device you didn't create without asking: `root set` cold-boots it.
- Don't turn on auto-grant all, or tap Grant on an app's su prompt, without the user's explicit say-so.
- Don't bypass Magisk's prompt on Play Store images.
- Don't install Magisk modules or CA certificates the user didn't give you.
- Don't leave a proxy, a CA or auto-grant behind on the user's device: clear what you set, and say what you changed.
