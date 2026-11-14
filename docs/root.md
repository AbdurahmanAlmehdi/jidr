# Root modes

Jidr roots your own Android emulators. Root is a first-class device property
(`root_mode: none | adb | magisk`), switched with `jidr root set` and applied
per device — the shared SDK system images are never modified.

| Mode | Images | Mechanism | Verify |
|---|---|---|---|
| `none` | all | stock launch | `adb shell id` → uid 2000 |
| `adb` | `default`, `google_apis`, `aosp_atd` | `adb root` after every boot | `adb shell id` → uid 0 |
| `magisk` | all | patched per-device ramdisk, cold boot | `adb shell su -c id` → uid 0, `su -c 'magisk -v'` |

```bash
jidr root set <device> magisk   # one-click full root
jidr root set <device> adb      # shell root
jidr root set <device> none     # revert either to stock
jidr root writable-system <device> on   # adb mode: a writable /system
jidr root verify <device>       # check the live uid against the mode
jidr root status <device>       # stored mode, health, Magisk version
```

> Play Integrity cannot pass on any emulator; Zygisk/DenyList only hides root
> from local checks.

## How Magisk mode works

`root.set magisk` runs the whole setup as one job, with progress and cancel:

1. **Fetch** the pinned Magisk release from the official GitHub release and
   check its SHA-256 (see [below](#magisk-licence)).
2. **Hash** the image's pristine `ramdisk.img`.
3. **Patch** a *per-device* copy on the host with `jidr-cpio`: `init` becomes
   `magiskinit`, the compressed payload goes under `overlay.d/sbin`, and the
   config is recorded in `.backup/.magisk`. The result is written to
   `<avd>/jidr/ramdisk.magisk-<ver>.img` atomically (temp file + rename).
4. **Cold boot** with `-ramdisk <patched>`.
5. **Install** the Magisk app (the release APK is also the manager app).
6. **Set up** `/data/adb/magisk`, then **reboot once** so `magiskd` enables su.
7. **Grant** the adb shell su automatically, then **verify**.

On `google_apis` (and `default` / `aosp_atd`) images this is fully zero-touch:
`adb root` is used to write Magisk's su-policy row for the shell, then adbd is
dropped back so the final check exercises Magisk's own `su`. Apps still get
Magisk's normal per-app prompt; the opt-in "auto-grant all" toggle is off by
default.

Before each cold boot, Jidr asks Android to shut down (`svc power shutdown`)
and waits for the guest to power off. Stopping only the emulator would drop
what Android hasn't written yet, such as the `AndroidWifi` network a new
device's first boot adds, which Wi-Fi saves 10 s later (#78).

## Revert

`root.set none` (or `adb`) on a Magisk device clears `launch.ramdisk`, deletes
the patched file and its metadata, and cold-boots the stock ramdisk. `none` and
`adb` then behave exactly as they do on a fresh device. A failed Magisk setup
leaves the device in the same revertable state (health `failed`, patched file
still present).

## Re-patch

A patched ramdisk records the pristine SHA-256, the Magisk version and APK,
and the build props it was built from. When the image or the version changes
(an image update, or a newer pinned Magisk), the next `root.set magisk` (or the
boot hook) rebuilds the patched copy from the pristine image; a props change
rebuilds it at once. Patches never stack.

## Offline Magisk

To use Magisk without network access, import the official APK once:

```bash
jidr root set <device> magisk --magisk /path/to/Magisk-v30.7.apk
```

The CLI adds the APK to the download cache by SHA-256 and `root.set` uses that
cached copy (`magisk_sha256`) instead of downloading. The SHA-256 is still
checked, so a tampered or wrong file is rejected.

## Auto-grant all (D22)

On a Full-root device the adb shell is always auto-granted `su`. Every other
app gets Magisk's normal allow/deny prompt — unless you opt in per device:

```bash
jidr root auto-grant <device> on    # grant every app root without a prompt
jidr root auto-grant <device> off   # back to the prompt (off by default)
```

`root.auto_grant` stores the flag on the device and, when it is running, sets
Magisk's superuser *automatic response* (`su_auto_response`: `2` = grant,
`0` = prompt) over `adb root`, reading the value back to confirm it. On a
stopped device it stores the flag and applies it on the next boot. Leaving Full
root resets the flag to off. The flag is off by default; Play Integrity is
unaffected.

## Build props (Phase 9f)

A Full-root device can report another identity: `Build.MODEL`, `BRAND`,
`MANUFACTURER`, `DEVICE`, `FINGERPRINT` and `VERSION.SECURITY_PATCH`. In the
GUI, open the player's widget panel → More → Developer tools → Build identity.
From the CLI or an agent:

```bash
jidr call device.props.set '{"device":"Phone","props":{"model":"Pixel 8","brand":"google","security_patch":"2026-10-05"}}'
jidr call device.props.get '{"device":"Phone"}'
jidr call device.props.set '{"device":"Phone","props":{}}'   # back to stock
```

`props` is the whole set: a field left out (or empty) keeps the image's value.
Values are trimmed, at most 256 characters, with no line breaks, and
`security_patch` is `YYYY-MM-DD`. A refused value names its field in
`error.data.field`.

How it works: the emulator's `-prop` only reaches `qemu.*` properties, so the
values are applied by Magisk's `resetprop` at boot. `device.props.set` stores
them in `<avd>/jidr/props.json` and re-patches the per-device ramdisk with the
same Magisk APK. The patch adds `overlay.d/jidr-props.rc`, which Magisk appends
to `init.rc`, and a script in `overlay.d/sbin`. At `post-fs-data`, before
zygote starts, the script runs `resetprop -n` for each property and its
partition copies (`ro.product.model`, `ro.product.vendor.model`, …). Only
identity properties are set, never hardware or HAL ones.

The next start applies them, as a cold boot; `restart_required` is true until
then. Other modes refuse `device.props.set` with `usage`. Reverting root boots
the stock ramdisk, so the stock values come back, and the stored props are
dropped. Play Integrity is unaffected: a changed fingerprint doesn't make an
emulator pass it.

## Writable /system (adb mode)

A shell-root (`adb`) device on API 31 or later can boot with a writable
`/system`, so you can push files into the system partitions
(`adb push … /system/…`):

```bash
jidr root writable-system <device> on    # -writable-system + adb remount
jidr root writable-system <device> off   # back to the read-only stock image
```

`root.writable_system {device, enabled}` stores the setting in the device's
root state (`writable_system`). Changing it forces a cold boot (D29): a running
device is restarted at once; a stopped one applies it at its next start.

It needs API 31 or later. Older images are refused with `usage` (exit code 2):
on API 30, `adb remount` leaves `/system` read-only and the emulator doesn't
come back from the reboot that would fix it.

How it works: with the setting on, the device launches with the emulator's
`-writable-system` option, which boots a writable copy of the system image.
After every boot the root hook runs `adb root`, then `adb remount`. The first
remount on a device needs a reboot before `/system` becomes writable (it
turns verity off and asks for one). If `/system` is still read-only after a
remount, Jidr reboots the guest once, waits for the boot to complete, roots
adbd again and remounts. It
then checks that a file can be created under `/system`, and the command fails
with `root_failed` (exit code 7) if it can't. Later boots only remount.

Revert: turning it off on a running device runs `adb enable-verity`, which
drops the remount overlay, then cold-boots without `-writable-system`, so the
device sees the stock read-only system image again. A device turned off while
stopped gets the same teardown at its next boot if an overlay is still mounted.
Leaving adb root (`root.set none` or `magisk`) turns the setting off the same
way.

Only `adb`-mode devices on images that allow `adb root` (`default`,
`google_apis`, `aosp_atd`) qualify. Turning it on for a `none` or `magisk`
device, or a Play Store image, is refused with `usage`; turning it off always
works. On a Full-root device, change system files with a Magisk module instead.
A device with a writable `/system` can't be made into a pool base (D42); turn
it off first. The writable copy of the system image can take several GB in the
device's folder.

## CA certificates (system trust)

To read an app's HTTPS traffic in Burp or mitmproxy, the app must trust the
proxy's CA. Apps that target Android 7 or later ignore user-installed CAs, so on a
Full-root device Jidr adds the CA to the **system** store instead:

```bash
jidr cert install <device> ~/Downloads/cacert.der   # PEM or DER; restarts the device
jidr cert list <device>                             # Jidr's CAs and whether each is trusted
jidr cert verify <device>                           # fails (exit 7) unless all are in the store
jidr cert remove <device> a4ac454c.0                # or a subject hash, a SHA-256, or --all
jidr proxy set <device> 8080                        # then send traffic to 10.0.2.2:8080
```

Burp exports its CA from Proxy › Proxy settings › Import / export CA
certificate (DER); mitmproxy keeps it in `~/.mitmproxy/mitmproxy-ca-cert.pem`.
In the GUI: the player's widget panel → More → Developer tools → CA
certificates → Add….

How it works: Jidr writes its own Magisk module, `/data/adb/modules/jidr_cacerts`,
in Magisk's documented module layout. The certificate is stored under the name
Android's store uses, `<subject hash>.<n>` (OpenSSL's `subject_hash_old`; `n`
counts certificates with the same subject, so a re-generated Burp CA becomes
`.1` next to the old one).

- **API 29–33** read `/system/etc/security/cacerts`. The module's `system/`
  folder puts the certificate there through Magisk's system overlay.
- **API 34+** read the Conscrypt APEX store, `/apex/com.android.conscrypt/cacerts`,
  which no overlay reaches. The module's `post-fs-data.sh`, which Magisk runs
  before Zygote starts, copies the store and Jidr's certificates into a
  tmpfs directory and bind-mounts it over the APEX store. Zygote, and every
  app it forks, inherits the mount.

A change applies on the next boot, so `install` and `remove` cold-boot the
device, then verify. Verification works like `su -c id` for root: it reads the
store as Zygote sees it (`nsenter` into Zygote's mount namespace) and compares
each certificate byte for byte. `list` reports `trusted: false` for a
certificate that is not in that store yet.

Installing a certificate that is already trusted changes nothing and doesn't
restart. Removing the last one deletes the module. Reverting root to `none`
or `adb` stops Magisk, so the certificates stop applying. Going back to Full
root (on the same data) brings them back. Wiping the device's data removes
them.

Only Full-root (Magisk) devices are supported: `adb root` can't change the
system store of a running device. An app that pins its certificates still
refuses the proxy; that needs Frida or similar on top.

## Flashable zips

`jidr flash <device> <zip>` and a zip dropped on the player both run
`file.flash` ([ADR-0004](decisions/0004-flashable-zips.md)). Jidr reads the
zip's file list on the Mac first, then:

| The zip | Full-root (`magisk`) device | Other devices |
|---|---|---|
| Magisk module (`module.prop` at the top) | `magisk --install-module`, a cold boot, then a check that Magisk loaded it from `/data/adb/modules/<id>` | Copied to `/sdcard/Download`, with a note that modules need Full root |
| Zygisk module (a module with `zygisk/`, e.g. LSPosed) | Same as a module. Magisk turns Zygisk on by default on emulators; if it was turned off in the Magisk app, the result says the module won't load until it's back on | Copied to Download |
| Riru module (`riru/`) | Refused: Magisk skips Riru modules while Zygisk is on, and Riru is discontinued. Use the module's Zygisk build | Refused |
| Recovery zip (TWRP flashables, GApps, ARM translation, custom kernels) | Refused: the emulator has no recovery, and these write the system partitions. Google APIs images already have Play services; arm64 images run ARM apps natively | Refused |
| OTA or factory image (`payload.bin`, partition `.img` files) | Refused: no fastboot or partition flashing on the emulator. Create a device from another system image instead | Refused |
| Anything else | Copied to Download | Copied to Download |

A refusal exits 2 (`usage`) with `error.data.kind`. Two module mistakes are
caught before the device is touched, each with the fix: `module.prop` inside a
folder of the zip (zip the folder's contents, not the folder), and an id that
isn't a valid Magisk id or starts with `jidr_` (Jidr's own modules).
`jidr push` still copies any zip to the device.

Remove a module in the Magisk app (Modules → Remove), then restart the device;
Magisk deletes it at the next boot. `root.module.install` stays as the explicit
"install this as a module" command, with the same checks.

## Magisk bases

A Full-root device can be made into a pool base (D42, Phase 13f), so agents
can lease throwaway rooted clones:

```bash
jidr base create <device> <base>   # the device must be running and rooted
jidr --json pool get <base>        # su -c id → uid 0 on the clone's serial
```

`base create` first checks the source: its patched ramdisk must exist and the
adb shell's `su -c id` must answer uid 0 without a prompt (`root_failed`
otherwise), because every clone resumes that state. The base keeps its own
copy of `<avd>/jidr/ramdisk.magisk-<ver>.img`, and each `pool get` clone gets
another copy (copy-on-write, so it costs no space until written) with
`launch.ramdisk` pointing at it. No two devices boot the same file, and the
SDK's `ramdisk.img` is never touched. The root mode, the Magisk version and
Magisk's settings (auto-grant included) carry over from the snapshot. A clone
resumes in seconds with `magiskd` already running, and the boot hook never
re-patches it: a clone always runs its base's Magisk version, even after a
newer one is pinned. Make a new base to move to it.

On a Play Store image the shell's grant is the one the person gave on the
source in the guided tap below. Choose **Forever** in Magisk's prompt before
making the base. A grant with a time limit runs out in the clones, and Jidr
never grants it for them (ADR-0002).

## On-device helper

Some features need code that runs as root *inside* the guest. Jidr has one
program for all of them, `jidr-helper` ([ADR-0003](decisions/0003-on-device-helper.md),
accepted as D44). The daemon pushes it to `/data/local/tmp/jidr-helper` and
starts it as root the first time a command needs it: in the adb shell on an
`adb` device, through `su` on a `magisk` device. A `none` device is refused
with `usage`. Nothing is installed: after a reboot the next command starts it
again.

```bash
jidr root helper <device>        # start it if needed; version, uid, SELinux context
jidr network loss <device> 10    # drop 10% of outgoing packets
jidr network dns <device> slow   # delay every DNS query 3000 ms (--delay-ms MS)
jidr network dns <device> fail   # drop every DNS query: lookups time out
jidr network loss <device>       # show loss and DNS on each interface
jidr network dns <device> normal # DNS back to normal
jidr network loss <device> 0     # remove the loss
```

Packet loss (`network.shaping.set {device, loss_pct}`) uses Linux traffic
control: `tc qdisc replace dev <iface> root netem loss <n>%` on every
interface with a default route (Wi-Fi `wlan0` and mobile data), so it applies
to what the device sends, whichever network apps use. `0` removes Jidr's
netem and the kernel's default qdisc comes back.

Bad DNS (`network.shaping.set {device, dns, dns_delay_ms}`) has two modes:
`slow` delays each DNS query by `dns_delay_ms` (default 3000 ms, Genymotion's
"bad DNS" profile), and `fail` drops every query, so lookups get no answer
and fail when Android's resolver gives up. It uses the same `tc` and the same
interfaces: the root becomes a two-band `prio` qdisc (handle `1d:`), `u32`
filters send UDP and TCP port 53 and DNS over TLS (853), over IPv4 and IPv6,
to band 1, and band 1 has its own netem (`delay <ms>`, or `loss 100%` for
`fail`). Everything else goes to band 2, which carries the packet loss when
there is one. Loss and DNS are independent: a `set` with only one of them
keeps the other.

Why `tc` and not iptables: a delay on DNS alone needs traffic control anyway,
and with `fail` as one more netem on the same band, one mechanism reads back
and clears both, with nothing left in netd's iptables chains. iptables
(`-j DROP` or `REJECT` on port 53, or DNAT to an unreachable resolver) could
only make DNS fail, never slow, and `REJECT` fails lookups at once, which
isn't what a bad network does. A DNS proxy in the helper would need it to
keep state and the device's DNS servers, against ADR-0003's stateless helper.
Not covered: an app doing DNS over HTTPS itself (port 443), and Private DNS
in strict mode through a DoH server, both look like any other HTTPS traffic.

Both live in the guest kernel, and Jidr also stores them on the device
(`network_shaping` in `device.list`) and re-applies them after every boot,
cold boots included (D44). After a boot the hook waits up to 60 s for Wi-Fi or
mobile data to have a route, and changes nothing when the kernel already has
the setting (a quick boot, or a snapshot taken with it). Restoring a named
snapshot brings back the kernel's shaping from when it was taken, until the
next cold boot re-applies the stored one. Setting the loss to `0` and DNS to
`normal` clears the kernel and forgets the stored value. An interface that
gets its route later (mobile data turned on afterwards) is shaped by the next
`jidr network …` command or boot.

If a device's root mode becomes `none`, Jidr keeps the stored setting but
skips it at boot with a warning (the `device.hook_failed` event, in the
daemon log), since nothing can apply it without root. Root the device again
and it comes back; or forget it with `jidr network loss <device> 0` and
`jidr network dns <device> normal`, which work without root on a `none`
device.

Both need API 31 or later. API 30's emulator kernel has no netem, so the
command fails there with `usage` ("this image's kernel has no netem") and
changes nothing (D44).

How it's reached: the helper listens on the abstract Unix socket
`@jidr-helper` (no file, no port on the guest's network) and serves only
root and the shell user, which `adb forward` connections come from; apps
are refused. The daemon connects through
`adb forward tcp:0 localabstract:jidr-helper`. It only keeps a helper whose
SHA-256 matches the build it ships, and replaces any other.

Development builds look for the helper in `target/jidr-helper/<arch>/`;
build it with `cargo xtask helper` (no Android NDK needed).

## Play Store images (beta)

`google_apis_playstore` images refuse `adb root`, so the shell grant cannot be
pre-approved. Jidr uses the guided tap from ADR-0002:

1. Full-root setup runs as above (patch, cold boot, app install, environment).
2. Jidr shows "Magisk will ask to grant root to Shell. Tap Grant." and runs
   `su -c id`, which raises Magisk's normal prompt.
3. It waits up to 60 s. Tap Grant, and the shell is granted root.
4. Denied or timed out, the device reports `awaiting_user_grant` with
   **Try again** and **Revert**.

There is no headless bypass on Play Store images; apps keep Magisk's per-app
prompt.

## Magisk licence

Magisk is **GPL-3.0-only**, so Jidr never bundles or redistributes it. It is
downloaded at runtime from
[topjohnwu/Magisk](https://github.com/topjohnwu/Magisk), its SHA-256 is checked
against the pin in `assets/magisk-pins.toml`, and the licence and source link
are shown before the first download. The same clean-room rule applies to the
patch itself: it reproduces Magisk's documented ramdisk layout without copying
its GPL code (see `docs/clean-room.md`).
