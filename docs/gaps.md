# Gaps

> **Last updated:** 2026-10-13 · **Owner:** Abdurahman (Part A), stream owners (Part B)
>
> Three lists live here:
> - **Part A: Research findings.** Features where public sources don't show how Genymotion does it (🔎 NEEDS RESEARCH in the plan's parity map). No agent builds any of these until its row is **Accepted** (plan decision D11).
> - **Part B: Cross-stream requests.** Things one work stream needs from another stream's code.
> - **Part C: Design opportunities.** Our own design ideas surfaced by the research.

## Rules of evidence

Sources allowed:

- Genymotion's public docs, release notes, marketing pages, public videos and public forum or Q&A posts;
- AOSP source and docs, the Android emulator's public protos and docs;
- QEMU docs;
- Magisk docs (reading only, never copying);
- behaviour you observe yourself on the AOSP emulator.

**Never** decompile, disassemble, string-dump or otherwise inspect Genymotion binaries, images or network traffic. That includes reverse-engineering tools such as REA, the android-reverse-engineering skill, Ghidra and Hopper. If the only way to learn something is from their binaries, write "Not public" and stop.

Cite everything. Every claim needs a URL, or a command and its output from your own test on the AOSP emulator. Quotes stay under 15 words. Paraphrase the rest.

Separate what Genymotion does from what Jidr should do. "How Genymotion does it" is evidence. "Jidr approach" is our own design, based on the AOSP emulator and Android.

Prove it on the AOSP emulator when you can. A test command that shows the mechanism working (e.g. `adb emu ...`, `adb shell setprop ...`, a gRPC call) moves Confidence to High.

Don't change the status column to Accepted. Only Abdurahman accepts a finding. Researchers set Researched.

Keep the summary table and the detail sections in sync. After every edit, bump "Last updated" above and add a row to the revision history at the bottom.

> **Note (2026-10-13 research pass):** the proofs below are **proposed verifications that have not been run**; this pass had no live emulator. They are written so they can be pasted in and confirmed. Confidence is High only where an official AOSP/Android doc states the mechanism. Anything resting on an untested command stays Medium or Low until someone runs it. Nothing here comes from Genymotion binaries.

> **Plan constraints (added at review, 2026-10-13). Every "Jidr approach" must respect these. Approaches below that said otherwise have been corrected.**
> 1. **Jidr never builds or modifies Google's system images.** Images come from `sdkmanager` and stay pristine. The only per-device modification is a patched copy of `ramdisk.img`, launched with `-ramdisk` (plan: Root engine, Phase 9d). "Edit build.prop in our image" or "custom HAL in our system image" is not available. The equivalents are (a) a Magisk module or `overlay.d` boot script on a Magisk device, (b) runtime changes over `adb root`, or (c) an on-device helper (G3). This generally raises effort and pushes those items post-MVP.
> 2. **Magisk is never bundled or baked into anything we ship** (GPL-3.0). It's downloaded from official releases at runtime, verified by SHA-256 and patched per device (D30, Phase 9d).
> 3. **Display:** `streamScreenshot` with inline RGBA frames is the plan (ADR-0001, D20). WebRTC is a fallback only.

## Legend

**Feasibility:**
- ✅ native: an existing emulator gRPC, console or adb call does it;
- 🟡 needs root: works through adb root or Magisk (`resetprop`, `settings`, `setprop`);
- 🟠 custom work: needs our own on-device helper, a Magisk module, or a QEMU flag the emulator doesn't expose;
- ❌ not feasible on the AOSP emulator.

**Effort:** S (≤1 day) · M (2–5 days) · L (>1 week), in agent-days.

**Confidence:** High (tested or officially documented) · Medium (credible secondary source) · Low (inference).

**Status:** Open → Researched → Accepted / Rejected / Deferred.

**Target:** MVP+ (add right after the MVP) · Post-MVP · Drop · Covered (already in a plan phase).

---

## Part A: Research findings

### Summary

| ID | Feature | Genymotion tier | Feasibility | Effort | Confidence | Target | Status |
|---|---|---|---|---|---|---|---|
| R1 | Baseband: operator and SIM identity (MCC/MNC, operator name, MSIN, phone number) | PRO | 🟠 (✅ phone number, proven) | M | High | Post-MVP | Accepted |
| R2 | Network profiles with packet loss and bad DNS (`4g-high-losses`, `4g-bad-dns`) | PRO | 🟡 needs root (proven: `tc netem` on `wlan0`) | S–M | High | Post-MVP | Accepted |
| R3 | Wi-Fi signal strength levels | PRO | 🟠 (✅ mobile bars, proven) | M | High | Phase 8a (mobile) · Post-MVP (Wi-Fi) | Accepted |
| R4 | Live IMEI / MEID / Device ID change | Free | 🟠 (✅ Android ID with root, proven) | L | Medium | Post-MVP | Accepted |
| R5 | Build props at creation (MODEL, BRAND, MANUFACTURER, DEVICE, FINGERPRINT, …) | Free | 🟡 Magisk devices only (`-prop` disproven) | M | High | MVP+ (Phase 9f) | Accepted |
| R6 | Disk I/O read-rate limit (High / Mid / Low / custom) | Free | ❌ in guest (proven) · 🟠 host QEMU route unverified | M–L | High (negative) | Post-MVP (moved from MVP+) | Accepted |
| R7 | Camera / media injection from image and video files | PRO | 🟠 (✅ webcam / still scene, proven) | L | High (native parts) | Phase 8a (webcam) · Post-MVP (files) | Accepted |
| R8 | Biometric scenarios (recognised, unrecognised, dirty, partial, insufficient, too fast) | All products | 🟠 (✅ recognised/unrecognised, proven) | S native · M rich | High | MVP: Phase 8a (native, D36) · Post-MVP (rich) | Accepted |
| R9 | Gamepad passthrough | PRO | 🟠 | M | Low | Post-MVP | Accepted |
| R10 | Device Link (a real phone mirrors and forwards its sensors) | ? | 🟠 | L | Low | Drop | Rejected |
| R11 | Superuser allow/deny prompt app in non-Magisk root (Android 12+) | PRO | ✅ via Magisk | — | Medium | Covered (Phase 9d) | Accepted |
| R12 | Flashing a dropped `.zip` (semantics, what's allowed) | Free | 🟡 (Magisk module) | S–M | Low | MVP+ (D35) | Accepted |
| R13 | Multitouch gestures with mouse and modifiers (exact bindings) *(optional)* | Free | ✅ (player) | S | Medium | Covered (Phase 7c) | Accepted |
| R14 | Default in-device keyboard shortcuts *(optional)* | Free | ✅ (player) | S | High | Covered (Phase 7c) | Accepted |
| R15 | Emulator WebRTC `v2.Rtc` service: proto location, codecs, latency (display fallback) | n/a | ✅ | L | Medium (Rtc untested; `streamScreenshot` measured) | Not needed (ADR-0001 passed) | Accepted |

### Proof run (2026-10-13)

Run on a throwaway arm64 AVD, `system-images;android-31;google_apis;arm64-v8a` (Android 12, userdebug), emulator 35.6.11 on Apple Silicon. Image facts are in `docs/research/2026-10-13-image-facts-api31-arm64.md`. The AVD was deleted afterwards.

| ID | Result | Key output |
|---|---|---|
| R1 | Phone number ✅. Operator change via property ❌: the property changes but telephony ignores it. The console has no operator command. | `adb emu phonenumber 15551234567` → `OK`. `setprop gsm.operator.alpha JidrMobile` → `getprop` shows JidrMobile, but `dumpsys telephony.registry` still shows `mOperatorAlphaLong=T-Mobile`. `emu help gsm` lists no operator sub-command. |
| R2 | ✅ Packet loss works with root. | `tc qdisc replace dev wlan0 root netem loss 30%` → ping went from 0% loss (10/10) to **25%** (15/20). `CONFIG_NET_SCH_NETEM=y` and `CONFIG_IFB=y` (`ifb0`/`ifb1` exist for incoming traffic). Default route is via `wlan0`. `network speed edge` / `delay gprs` → `network status` shows 473600 bit/s and 35–200 ms. |
| R3 | ✅ Mobile bars. Wi-Fi strength is fixed. | `gsm signal-profile 0/2/4` → WCDMA `ss=-105/-81/-53`, `level=1/3/4`. Wi-Fi stays at `rssi=-50` in `dumpsys wifi`. |
| R4 | ✅ Android ID with root. IMEI read-only. | `settings put secure android_id 1234567890abcdef` → read back the same value. IMEI readable via `service call iphonesubinfo 1` (15 digits). No IMEI property or setter found. |
| R5 | ❌ `-prop` route; only Magisk `resetprop` remains. | Launched with `-prop ro.product.model=JidrOne -prop persist.jidr.test=1` → `ro.product.model=sdk_gphone64_arm64`, and `persist.jidr.test` is empty. |
| R6 | ❌ Throttling inside the device is impossible on this kernel. | `/dev/blkio` has only `blkio.bfq.*` stats, with no `throttle.*` files; writing `read_bps_device` gives "No such file". `CONFIG_BLK_DEV_THROTTLING` is not set, `CONFIG_DM_DELAY` is not set, and the cgroup v2 mount has no controllers. Reads stayed at 420–583 MB/s. The only remaining route is host-side QEMU drive throttling via `-qemu` passthrough (unverified). |
| R7 | ✅ Native parts. | `emulator -webcam-list` → `webcam0` and `webcam1`. `emu help virtualscene-image` → `virtualscene-image <wall\|table> [path-to-image]`. |
| R8 | ✅ Native touch and remove. | `emu finger touch 1` → `OK`; `emu finger remove` → `OK` (no fingerprint enrolled; the command is accepted). |
| R12 | Not run: the stock image has no Magisk. To be proven in Phase 9d. | `which magisk` → not found. |
| R15 | ✅ `streamScreenshot` measured; `Rtc` not tested. | Python `grpcio`, token auth from the discovery file. **Downscaling needs both width and height** (width alone returns the full 1080×2340). Inline RGBA: **30.1 fps** at 540×1170 (2.5 MB/frame), **24.4 fps** at full size (10.1 MB/frame), 0 dropped frames. The client must raise gRPC's max receive size (the 4 MB default fails). |

<!-- Findings go below this line, one section per ID, in ID order. -->

---

### R1: Baseband: operator and SIM identity

- **Status:** Accepted (D37)
- **Genymotion tier:** PRO
- **What the user sees in Genymotion:** A Baseband widget with two groups, Network Operator (MCC/MNC, name) and SIM Operator (MCC/MNC, MSIN, name), plus a phone number field. The values show up in the device's network and SIM settings. Calls and SMS to real phones aren't possible; everything is emulated.
- **How Genymotion does it:** Not public. The widget's behaviour is documented; the mechanism isn't.
- **Evidence (Genymotion):**
  - https://docs.genymotion.com/features/baseband/: the Baseband widget fields are network and SIM MCC/MNC, MSIN, name and phone number; operator and SIM are emulated only.
  - https://docs.genymotion.com/desktop/05_Genymotion_Shell/: Genymotion Shell has `phone` and `gsm` command groups.
- **Jidr approach:**
  - The AOSP emulator's modem is a software GSM modem (`android_modem`).
  - The phone number can be set directly with the `phonenumber` console command or the `setPhoneNumber` gRPC call.
  - Operator name and MCC/MNC are **not** exposed by any console command. Matching Genymotion needs an on-device helper (root, G3) that sets the telephony properties and makes the RIL re-read them.
  - A modem-config patch is not an option (plan constraint 1).
  - So the phone number is native, and operator/SIM identity is custom work.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  # image: system-images;android-34;google_apis;arm64-v8a ; emulator 35.x
  adb emu phonenumber 15551234567          # documented: sets device phone number
  adb shell getprop | grep gsm.sim          # inspect which operator props telephony reads
  # operator/MCC-MNC live-change path (needs adb root image) is still to be verified
  ```
- **Feasibility:** 🟠 custom work (✅ native for the phone number only)
- **Requires:** an image that allows `adb root` (not Play Store) for operator and MCC/MNC; the phone number works on any image.
- **Effort:** M
- **Confidence:** Medium
- **Risks / caveats:**
  - The emulator's modem is not a real baseband, so some fields may not reach every telephony surface.
  - Persistence across snapshots and reboots needs checking.
  - Play Store images block `adb root`.
- **Recommended target:** Post-MVP. SIM/operator spoofing is a testing nicety, not core to a rooted-device MVP.
- **Protocol / CLI sketch:** `baseband.set {networkMccMnc, networkName, simMccMnc, simMsin, simName, phoneNumber}` · `jidr baseband set --sim-operator 60201 --name "Jidr Mobile"`
- **Open questions:**
  - Can MCC/MNC change live on a stock AOSP modem without a reboot, or only at boot?
  - Does the image type (`google_apis` vs `aosp`) change which properties telephony honours?

---

### R2: Network profiles with packet loss and bad DNS

- **Status:** Accepted (D37)
- **Genymotion tier:** PRO
- **What the user sees in Genymotion:** A Network widget with profiles. Two matter here: **4G (high packet losses)** is 10% loss, and **4G (high DNS delay)** is a 3000 ms DNS delay. Genymotion Shell has the same set: `network setprofile <…|4g-high-losses|4g-bad-dns|…>`.
- **How Genymotion does it:** Not public. The profiles and their numbers are documented; the shaping mechanism isn't.
- **Evidence (Genymotion):**
  - https://docs.genymotion.com/features/network/: 4G (High packet losses) is 10% loss; 4G (High DNS delay) is a 3000 ms DNS delay.
  - https://docs.genymotion.com/desktop/05_Genymotion_Shell/: `network setprofile` accepts `4g-high-losses` (10% loss) and `4g-bad-dns` (3000 ms DNS delay).
- **Jidr approach:**
  - The AOSP emulator console handles latency and bandwidth natively (`network delay`, `network speed`), but has **no packet-loss or DNS-specific setting**.
  - We add those ourselves with Linux traffic control inside the guest. An HTB class tree with `netem` handles rate, delay and loss. A separate class matching UDP/TCP port 53 adds a DNS-only delay.
  - This needs root in the guest, through `adb root` or the G3 helper.
  - Bandwidth and latency presets can reuse the native commands.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  adb emu network delay gprs                # documented native latency preset
  adb emu network speed edge                # documented native bandwidth preset
  # loss + DNS delay (our addition, needs root):
  adb root && adb shell 'tc qdisc add dev radio0 root netem loss 10%'   # to verify interface name
  ```
- **Feasibility:** 🟠 custom work (✅ native for speed and delay only)
- **Requires:** root in the guest for `tc`/`netem`; the guest interface names (`wlan0`, `radio0`) must be confirmed on our images (G4).
- **Effort:** M
- **Confidence:** Medium. High that the native console lacks loss and DNS controls; Medium that our `tc` path works cleanly on the AOSP image.
- **Risks / caveats:**
  - Interface names differ between images.
  - Applying `netem` to downloads needs an `ifb` redirect.
  - Restoring a snapshot must re-apply the shaping.
- **Recommended target:** Post-MVP.
- **Protocol / CLI sketch:** `network.setProfile {name}` / `network.setShaping {iface, downKbit, upKbit, delayMs, lossPct, dnsDelayMs}` · `jidr net profile 4g-bad-dns`
- **Open questions:** On recent emulators (the netsim path), does Wi-Fi traffic still pass through an interface we can shape with `tc`, or does netsim need its own hook?

---

### R3: Wi-Fi signal strength levels

- **Status:** Accepted (D37)
- **Genymotion tier:** PRO
- **What the user sees in Genymotion:** Signal strength levels (none, poor, moderate, good, great) for mobile and Wi-Fi. In Genymotion Shell: `network setsignalstrength mobile moderate`.
- **How Genymotion does it:** Not public. The control is documented; the mechanism isn't.
- **Evidence (Genymotion):**
  - https://docs.genymotion.com/desktop/05_Genymotion_Shell/: `network setsignalstrength mobile moderate` sets the mobile signal strength.
- **Jidr approach:**
  - For **mobile** bars, the AOSP console is enough: `gsm signal-profile <0-4>` or `gsm signal rssi <0-31>` changes the reported strength (documented).
  - For **Wi-Fi strength (RSSI)** there is no native control; the simulated Wi-Fi radio reports a fixed signal. A changing Wi-Fi bar needs custom work, either a Wi-Fi medium simulator such as `wmediumd` or a small hwsim parameter. Both still need investigating.
  - So mobile is native, and Wi-Fi is custom and unconfirmed.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  adb emu gsm signal-profile 1     # documented: 0..4 signal profile -> mobile bars
  adb emu gsm signal rssi 7        # documented: rssi 0..31
  # Wi-Fi RSSI change: no documented command found — open.
  ```
- **Feasibility:** 🟠 custom work (✅ native for mobile bars)
- **Requires:** nothing for mobile; a Wi-Fi medium simulator for Wi-Fi strength (to be determined).
- **Effort:** M (mostly the Wi-Fi side)
- **Confidence:** Medium
- **Risks / caveats:** Wi-Fi strength may not be changeable at all without patching the emulated Wi-Fi stack. Confirm before promising it.
- **Recommended target:** mobile bars are in **Phase 8a (D36)**; Wi-Fi strength is Post-MVP.
- **Protocol / CLI sketch:** `signal.set {radio: mobile|wifi, level}` · `jidr signal mobile poor`
- **Open questions:** Can the emulator's Wi-Fi stack (hostapd/hwsim or netsim) be told to report a given RSSI to Android?

---

### R4: Live IMEI / MEID / Device ID change

- **Status:** Accepted (D37)
- **Genymotion tier:** Free
- **What the user sees in Genymotion:** An Identifiers widget that shows the Android ID and the Device ID (IMEI/MEID) and lets the user change them.
- **How Genymotion does it:** Not public.
- **Evidence (Genymotion):**
  - https://docs.genymotion.com/saas/07_Features_and_emulated_sensors/: the Identifiers widget shows the Android ID and the Device ID (IMEI/MEID).
- **Jidr approach:**
  - No native AOSP console or gRPC command sets the IMEI; the emulator's IMEI comes from the emulated modem.
  - Changing it live needs an on-device helper (root, G3) that rewrites the value telephony reads and forces a re-query. Baking a value into the image isn't an option (plan constraint 1).
  - The Android ID is easy with root, since it's a settings value: `settings put secure android_id …`.
  - The hard part is changing the IMEI live, and that's unconfirmed.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  adb root
  adb shell settings get secure android_id    # Android ID is a settings value (changeable)
  # IMEI: no documented setter; modem-level path to verify
  ```
- **Feasibility:** 🟠 custom work
- **Requires:** an image that allows `adb root`; probably a modem helper for the IMEI.
- **Effort:** L
- **Confidence:** Low
- **Risks / caveats:**
  - The IMEI may only take effect at boot, not live.
  - Behaviour differs by Android version and by which telephony API an app queries.
- **Recommended target:** Post-MVP. The Android ID alone could be an S-sized MVP+ item.
- **Protocol / CLI sketch:** `identity.set {imei, meid, androidId}` · `jidr identity set --imei 490154203237518`
- **Open questions:** Is there any modem path on the stock AOSP emulator to change the IMEI without modifying the image?

---

### R5: Build props at creation (MODEL, BRAND, MANUFACTURER, DEVICE, FINGERPRINT, …)

- **Status:** Accepted (D37)
- **Genymotion tier:** Free
- **What the user sees in Genymotion:** Device identity fields set per virtual device when it's created.
- **How Genymotion does it:** Not public. They build their own images, so they can edit the properties at build time. Jidr can't (plan constraint 1).
- **Evidence:**
  - https://github.com/topjohnwu/Magisk/blob/master/docs/tools.md: `resetprop` changes system properties systemlessly, including read-only `ro.*` ones.
  - https://developer.android.com/studio/run/emulator-commandline: the emulator's `-prop` flag sets a system property at boot. Which properties it is allowed to set is unverified.
- **Jidr approach** (corrected at review; editing build.prop in the image is removed):
  1. **Magisk devices:** our per-device patched ramdisk already carries an `overlay.d` boot hook (Phase 9a/9d). It runs `resetprop` for identity properties early in boot. This is systemless and reversible: un-patching removes it.
  2. ~~**Any device:** the emulator's `-prop` at launch~~ **Disproven (2026-10-13 proof run):** `-prop ro.product.model=…` had no effect.
  3. **Root-only devices (`adb root`):** `setprop` works only for properties that can change at runtime, so it's not enough for `ro.product.*`.

  Limit this to identity properties (model, brand, manufacturer, device, fingerprint, security_patch). Never touch hardware or HAL properties.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  emulator @avd -prop ro.product.model="Jidr One"   # check whether ro.* is honoured
  adb shell getprop ro.product.model
  # on a Magisk device:
  adb shell su -c 'resetprop ro.product.model "Jidr One"' && adb shell getprop ro.product.model
  ```
- **Feasibility:** 🟡 on Magisk devices; ✅ if `-prop` turns out to accept `ro.*` properties
- **Requires:** Full root (Magisk) mode, unless `-prop` works.
- **Effort:** S if `-prop` works · M with the Magisk boot hook
- **Confidence:** Medium. The tools are documented, but this exact combination hasn't been tested on our images.
- **Risks / caveats:**
  - Changing hardware or HAL properties can stop the device booting.
  - A fingerprint should be paired with a matching `ro.build.version.security_patch`.
  - Play Integrity is a separate problem and out of scope.
- **Recommended target:** MVP+. It fits the rooted-device story.
- **Protocol / CLI sketch:** `device.props.set {model, brand, manufacturer, device, fingerprint, securityPatch}` · `jidr device props --model "Phone X" --fingerprint …`
- **Open questions:**
  - Does `-prop` accept `ro.*` properties on emulator 35.x?
  - Which property set does current Android read for each field on our images: `ro.product.*`, `ro.product.system.*`, or the vendor/odm copies?

---

### R6: Disk I/O read-rate limit (High / Mid / Low / custom)

- **Status:** Accepted (D37)
- **Genymotion tier:** Free
- **What the user sees in Genymotion:** A Disk I/O widget with device profiles: High 200 MiB/s, Mid 100 MiB/s, Low 50 MiB/s, or a custom read limit. Switching profile clears the disk cache. In Genymotion Shell: `diskio setreadratelimit <KB/s>` and `diskio clearcache`.
- **How Genymotion does it:** Not public. The limits and the cache-clearing behaviour are documented.
- **Evidence (Genymotion):**
  - https://docs.genymotion.com/saas/07_Features_and_emulated_sensors/: High 200 MiB/s, Mid 100 MiB/s, Low 50 MiB/s; switching profile clears the disk cache.
  - https://docs.genymotion.com/desktop/05_Genymotion_Shell/: `diskio setreadratelimit` sets the read rate in KB/s, `0` turns it off, and `clearcache` clears the cache.
- **Jidr approach:**
  - The emulator has no native setting for this. Instead we throttle inside the guest with the cgroup I/O controller.
  - Use `io.max` on cgroup v2 (preferred, O4) or `blkio.throttle.read_bps_device` on cgroup v1, applied to the block devices behind the system and data partitions. This needs root.
  - Clear the cache with `sync; echo 3 > /proc/sys/vm/drop_caches`.
  - Only read bandwidth is needed to match Genymotion. Write and IOPS limits are easy extensions in the same files.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  adb root
  adb shell 'echo "MAJ:MIN 52428800" > /dev/blkio/blkio.throttle.read_bps_device'  # v1, 50 MiB/s
  adb shell 'sync; echo 3 > /proc/sys/vm/drop_caches'
  # MAJ:MIN from /proc/mounts + /sys/block/<dev>/dev — to verify on our image (G4)
  ```
- **Feasibility:** 🟠 custom work (needs root and a helper)
- **Requires:** an image that allows `adb root`; the right cgroup controller in the emulator kernel (G4).
- **Effort:** M
- **Confidence:** Medium
- **Risks / caveats:**
  - cgroup v1 and v2 differ.
  - The throttle must be re-applied after a snapshot is restored.
- **Recommended target:** Post-MVP (moved from MVP+ after the proof run: the emulator kernel has no block throttling, so only a host-side QEMU route remains, and it's unverified).
- **Protocol / CLI sketch:** `diskio.setReadLimit {bytesPerSec}` · `diskio.clearCache` · `jidr diskio limit 50MiB`
- **Open questions:**
  - Does the AOSP emulator kernel use cgroup v1 (blkio) or v2 (io.max)?
  - Which block device backs `/` and which backs `/data`?

---

### R7: Camera / media injection from image and video files

- **Status:** Accepted (D37)
- **Genymotion tier:** PRO
- **What the user sees in Genymotion:** A Camera (and Microphone) injection widget. It feeds the device camera from a host webcam or from image, video or audio files, assignable to the front and/or back camera, with resizing.
- **How Genymotion does it:** Not public. The sources (webcam, image, video, audio) are documented; the pipeline isn't.
- **Evidence (Genymotion):**
  - https://rozz.genymotion.com/pages/emulated-sensors-and-features-genymotion-desktop.html *(secondary Q&A mirror)*: image, video and audio files can be used as camera and audio inputs, per front/back camera, with resizing.
- **Jidr approach:**
  - The AOSP emulator natively covers two sources.
    - A host **webcam** passthrough: `-camera-back webcam0`.
    - A **still image** placed in the virtual scene: `virtualscene-image wall|table <path>`, documented for AR and QR testing.
  - Feeding an **arbitrary video or image file as the raw camera feed** isn't native. A custom camera HAL is ruled out (plan constraint 1).
  - The realistic option is a **virtual webcam on the Mac**: a CoreMediaIO camera extension that plays the file, which the emulator then uses as `webcam0`.
  - This is the biggest build in the set.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  emulator @avd -camera-back webcam0        # documented: host webcam as back camera
  adb emu virtualscene-image table /path/barcode.png   # documented: still image into virtual scene
  # arbitrary video-as-camera: no native path — host virtual webcam
  ```
- **Feasibility:** 🟠 custom work (✅ native for the webcam and the still virtual-scene image)
- **Requires:** a Mac virtual-camera extension for file playback. The webcam and still-image paths need no custom work.
- **Effort:** L
- **Confidence:** Medium
- **Risks / caveats:**
  - A camera extension needs its own signing and entitlement.
  - The still-image path only places an image inside the virtual scene (for AR/QR); it isn't a clean full-frame camera.
- **Recommended target:** the host webcam is in **Phase 8a (D36)**; file/video injection is Post-MVP.
- **Protocol / CLI sketch:** `camera.setSource {camera: front|back, source: webcam|image|video, path}` · `jidr camera back --video clip.mp4`
- **Open questions:** Does the emulator pick up a CoreMediaIO virtual camera as `webcamN` on macOS?

---

### R8: Biometric scenarios (recognised, unrecognised, dirty, partial, insufficient, too fast)

- **Status:** Accepted (D37)
- **Genymotion tier:** Available for all products (was PRO)
- **What the user sees in Genymotion:** A Biometrics widget (Android 13+) with six scenarios: Recognized, Unrecognized, Dirty, Partial, Insufficient and Too fast. It also has a toggle to answer Recognized automatically. It needs one enrolled fingerprint.
- **How Genymotion does it:** Not public. The six scenarios are documented.
- **Evidence (Genymotion):**
  - https://docs.genymotion.com/saas/07_Features_and_emulated_sensors/: the six fingerprint scenarios and the "Recognized by default" toggle.
  - https://rozz.genymotion.com/pages/biometric-authentication-is-available-for-all-products.html *(secondary)*: the Biometrics widget is available across all Genymotion products.
- **Jidr approach:**
  - The AOSP console gives **recognised / unrecognised** directly (documented; also `sendFingerprint` over gRPC).
    - `finger touch <id>` simulates a matching finger.
    - A wrong id, or `finger remove`, simulates a non-match.
  - The richer states (**dirty / partial / insufficient / too fast**) aren't exposed. They correspond to the fingerprint HAL's `ACQUIRED_*` help codes.
  - A custom HAL is ruled out (plan constraint 1), so those would need a Magisk-module HAL override and stay Post-MVP.
  - Recognised and unrecognised can ship cheaply.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  # after enrolling a fingerprint in Settings:
  adb emu finger touch 1     # documented: simulate matching finger
  adb emu finger remove      # documented: finger lifted
  # dirty/partial/insufficient/too_fast: no native command
  ```
- **Feasibility:** 🟠 custom work (✅ native for recognised/unrecognised)
- **Requires:** nothing for touch and remove; a HAL override for the rich scenarios.
- **Effort:** S (native) · M+ (rich)
- **Confidence:** High for the native part (officially documented)
- **Risks / caveats:**
  - The biometric HAL interface (AIDL or legacy HIDL) depends on the API level.
  - An enrolled fingerprint is needed either way.
- **Recommended target:** recognised/unrecognised are in **Phase 8a (D36)**; the rich scenarios are Post-MVP.
- **Protocol / CLI sketch:** `biometrics.scenario {recognized|unrecognized|dirty|partial|insufficient|too_fast}` · `jidr finger touch`
- **Open questions:** Which biometrics HAL version ships in our target images, and what does each `ACQUIRED_*` code show to apps there?

---

### R9: Gamepad passthrough

- **Status:** Accepted (D37)
- **Genymotion tier:** PRO
- **What the user sees in Genymotion:** A host game controller drives the virtual device, and its buttons and sticks reach Android games.
- **How Genymotion does it:** Not public.
- **Evidence (Genymotion):**
  - https://rozz.genymotion.com/pages/emulated-sensors-and-features-genymotion-desktop.html *(secondary)*: mentions gamepad support (e.g. a PS5 pad on Windows maps to an Xbox 360 controller).
- **Jidr approach:**
  - The AOSP emulator's extended controls have no clearly documented gamepad passthrough.
  - Our route:
    1. The player reads the host controller (e.g. with SDL or `gilrs`).
    2. An on-device helper (G3) creates a virtual input device through `uinput`, declaring the real controller's USB vendor and product IDs so Android loads the matching key layout.
    3. Buttons and stick movements are then written as input events.
  - This is custom work and mostly unconfirmed on AOSP.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  adb shell getevent -lp        # does any pad device appear natively?
  # expected: none without our uinput helper
  ```
- **Feasibility:** 🟠 custom work
- **Requires:** root (for `/dev/uinput`) and a matching key layout in the image.
- **Effort:** M
- **Confidence:** Low
- **Risks / caveats:**
  - Each controller needs its own mapping.
  - HID and XInput differ on the host.
  - Key-layout files must exist for the declared IDs.
- **Recommended target:** Post-MVP.
- **Protocol / CLI sketch:** `input.gamepad.connect {name, vendor, product}` + event stream · `jidr gamepad attach`
- **Open questions:** Does a recent AOSP emulator build already forward host gamepads (through SDL), which would save us the uinput helper?

---

### R10: Device Link (a real phone mirrors and forwards its sensors)

- **Status:** Rejected (D37)
- **Genymotion tier:** ?
- **What the user sees in Genymotion:** A real phone acts as a touchscreen and motion-sensor source for the virtual device.
- **How Genymotion does it:** Not public.
- **Evidence (Genymotion):** Not found in current public docs. The earlier spec harvest listed it as a 3.8.0+ feature but couldn't confirm its tier. Treat it as "Not public".
- **Jidr approach:**
  - The AOSP emulator has no equivalent.
  - The closest open-source reference is scrcpy, which works in the other direction: a real phone mirrored to a computer.
  - For Jidr this would mean a companion phone app plus two `adb forward` channels: one sends a preview stream to the phone, the other carries the phone's touch and sensor events back into the virtual device.
  - It's large, niche, and not aligned with the core of an emulator on the Mac.
- **Proof on the AOSP emulator:** n/a. It would be a new subsystem, with nothing to prove against the stock emulator.
- **Feasibility:** 🟠 custom work
- **Requires:** a companion Android app, adb forwarding, and sensor/touch re-injection.
- **Effort:** L
- **Confidence:** Low
- **Risks / caveats:** A big surface for a feature few users ask of a desktop emulator.
- **Recommended target:** Drop. Revisit only if there's real demand.
- **Protocol / CLI sketch:** n/a (deferred)
- **Open questions:** Is there any user demand for this at all?

---

### R11: Superuser allow/deny prompt app in non-Magisk root (Android 12+)

- **Status:** Accepted (D37)
- **Genymotion tier:** PRO
- **What the user sees in Genymotion:** A superuser prompt that grants or denies root to apps on a rooted device.
- **How Genymotion does it:** Not public. (Their docs mention a bundled Superuser app but not which one.)
- **Evidence:**
  - https://docs.genymotion.com/features/root/: rooted devices show an allow/deny prompt when an app asks for root.
  - https://deepwiki.com/topjohnwu/Magisk/4.2-root-access-management *(AI-generated secondary summary; confirm against Magisk's own docs)*: MagiskSU checks root policy in a SQLite database and prompts the user through the Magisk app. Its `root_access` modes are Disabled, Apps Only, ADB Only, and Apps and ADB.
- **Jidr approach** (corrected at review):
  - **Already covered by the plan.**
  - In **Full root (Magisk)** mode, MagiskSU provides the allow/deny prompt and the policy store. We don't build our own superuser app. Jidr surfaces Magisk's state (D22: shell auto-granted, apps prompted, an opt-in "auto-grant all").
  - In **Shell root (adb)** mode there is deliberately no app-level `su`; only the adb shell is root. That's the documented difference between the two modes.
  - Magisk is **downloaded at runtime and patched into a per-device ramdisk copy**. It's never bundled or baked into an image (plan constraint 2, Phase 9d).
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  # on a Jidr Magisk device:
  adb shell su -c id         # shell is auto-granted (D22) -> uid=0
  ```
- **Feasibility:** ✅ via Magisk (Phase 9d)
- **Requires:** Full root (Magisk) mode.
- **Effort:** — (part of Phase 9d)
- **Confidence:** Medium
- **Risks / caveats:** DenyList and Zygisk are extra scope.
- **Recommended target:** Covered (Phase 9d). Optionally expose MagiskSU's `root_access` mode as a per-device setting.
- **Protocol / CLI sketch:** `root.policy.set {mode}` (maps to MagiskSU `root_access`) · `jidr root policy apps-and-adb`
- **Open questions:** None beyond Phase 9a's Play Store spike.

---

### R12: Flashing a dropped `.zip` (semantics, what's allowed)

- **Status:** Accepted (D37)
- **Genymotion tier:** Free
- **What the user sees in Genymotion:** Dropping a `.zip` onto the device flashes or applies it.
- **How Genymotion does it:** Not public.
- **Evidence:**
  - https://docs.genymotion.com/: dropping a zip that contains `/system` flashes it, and the docs advise cloning the device first. (From the earlier spec harvest; the exact page wasn't re-cited in this pass.)
  - https://topjohnwu.github.io/Magisk/guides.html: Magisk modules are the standard systemless package format, installed through the Magisk app or the `magisk --install-module` command.
- **Jidr approach:**
  - An emulator has no fastboot or recovery flashing in the phone sense, and Jidr keeps Google's system images pristine (plan constraint 1).
  - The bounded, useful version:
    - On a **Magisk** device, a dropped zip installs as a Magisk module with `magisk --install-module <zip>`, followed by a reboot.
    - On other devices, refuse it with an explanation, or push it to `/sdcard/Download` like any other file.
  - Full OTA or firmware zip flashing is out of scope.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  adb push module.zip /data/local/tmp/
  adb shell su -c 'magisk --install-module /data/local/tmp/module.zip'
  adb reboot
  ```
- **Feasibility:** 🟡 on Magisk devices (module install, not general flashing)
- **Requires:** Full root (Magisk) mode.
- **Effort:** S–M
- **Confidence:** Low
- **Risks / caveats:**
  - Users may expect TWRP-style flashing, so this must be scoped clearly in the UI.
  - A bad module can stop the device booting. Magisk's safe mode and our revert path cover that.
- **Recommended target:** MVP+. **Decided (D35):** Magisk devices install a dropped zip as a module; other devices get it pushed to `/sdcard/Download` with a note. Phase 7c updated.
- **Protocol / CLI sketch:** `module.install {path}` · `jidr module install foo.zip`
- **Open questions:** On a non-Magisk device, should a dropped zip be refused or pushed to `/sdcard/Download`?

---

### R13: Multitouch gestures with mouse and modifiers (optional)

- **Status:** Accepted (D37)
- **Genymotion tier:** Free
- **What the user sees in Genymotion:** Mouse plus modifier keys produce pinch and rotate multitouch in the device window.
- **How Genymotion does it:** Not public (a standard mapping in the player).
- **Evidence (Android):**
  - https://developer.android.com/studio/run/emulator-console: the emulator supports multitouch through `EV_ABS` slots (protocol B), e.g. a two-finger pinch using slots 0 and 1.
- **Jidr approach:**
  - **Covered by Phase 7c** ("multitouch with modifiers").
  - The player maps mouse plus a modifier to a two-point multitouch sequence mirrored around the pointer. That goes through gRPC `sendTouch` / `streamInputEvent` with two pointer ids; the console path isn't used.
- **Proof on the AOSP emulator:** n/a. Phase 7c's acceptance tests cover it.
- **Feasibility:** ✅ (player)
- **Effort:** S
- **Confidence:** Medium
- **Recommended target:** Covered (Phase 7c).
- **Open questions:** Which modifiers feel right on macOS (Option vs Command) without clashing with system shortcuts? Decide in Phase 7c.

---

### R14: Default in-device keyboard shortcuts (optional)

- **Status:** Accepted (D37)
- **Genymotion tier:** Free
- **What the user sees in Genymotion:** Keyboard shortcuts for hardware buttons and actions in the device window.
- **How Genymotion does it:** Not public.
- **Evidence (Android):**
  - https://developer.android.com/studio/run/emulator-extended-controls: the emulator has a documented set of default keyboard shortcuts.
- **Jidr approach:** **Covered by Phase 7c** (toolbar plus the command palette with shortcut hints). Define Jidr's own map, using the AOSP emulator's published one as a baseline.
- **Feasibility:** ✅ (player)
- **Effort:** S
- **Confidence:** High
- **Recommended target:** Covered (Phase 7c).
- **Open questions:** None.

---

### R15: Emulator WebRTC `v2.Rtc` service (display fallback)

- **Status:** Accepted (D37)
- **Genymotion tier:** n/a (an AOSP emulator capability, not a Genymotion parity item)
- **What the user sees:** A low-latency display stream from the emulator to the player.
- **How the AOSP emulator does it:** Public. The emulator exposes a gRPC `Rtc` service (WebRTC) alongside `streamScreenshot` on `EmulatorController`.
- **Evidence (AOSP):**
  - https://android.googlesource.com/platform/tools/base/+/studio-main/emulator/: the proto files include `emulator_controller.proto` and `rtc_service.proto`.
  - https://android.googlesource.com/platform/tools/base/+/7379a973bd0d25c58267b991ac03723fbee72727/emulator/proto/rtc_service_v2.proto: a v2 `Rtc` service in `android.emulation.control.v2` for setting up a WebRTC peer connection.
  - Local check from the earlier research: the RTC proto **isn't shipped** in the local SDK (`emulator/lib/`, emulator 35.6.11). It would have to be vendored from AOSP at a pinned revision.
- **Jidr approach** (corrected at review):
  - **The plan is `streamScreenshot` with inline RGBA frames into Slint** (ADR-0001, D20). It stays local-only and avoids a WebRTC client.
  - `Rtc` v2 is a **fallback** only, if Phase 1 misses ≥30 fps or ≤80 ms after trying MMAP and wgpu import.
  - If adopted, vendor `rtc_service_v2.proto` and `ice_config.proto` at a pinned revision into `jidr-proto`.
- **Proof on the AOSP emulator** *(proposed, not yet run):*
  ```bash
  emulator @avd -grpc 8556 -grpc-use-token -no-window
  TOKEN=$(grep grpc.token ~/Library/Caches/TemporaryItems/avd/running/pid_*.ini | cut -d= -f2)
  grpcurl -plaintext -H "authorization: Bearer $TOKEN" -import-path ~/Library/Android/sdk/emulator/lib \
    -proto emulator_controller.proto -d '{"format":"RGBA8888","width":540}' \
    localhost:8556 android.emulation.control.EmulatorController/streamScreenshot | head -c 300
  ```
- **Feasibility:** ✅ (public gRPC services)
- **Requires:** the emulator launched with `-grpc` and a token; RTC protos vendored from AOSP.
- **Effort:** L (WebRTC client plumbing)
- **Confidence:** Medium (the service is public; latency and codecs are unmeasured)
- **Risks / caveats:**
  - The proto headers say the service definition "might change without notice", so pin a revision.
  - A WebRTC client is non-trivial.
- **Recommended target:** **Not needed.** ADR-0001 passed with inline RGBA frames (Stream A, Phase 1). Revisit only if the player regresses.
- **Open questions:** Codecs offered and end-to-end latency on Apple Silicon. Measure only if ADR-0001 needs it.

---

## Part B: Cross-stream requests

Streams: **A** core (daemon, CLI, MCP, backend, SDK) · **B** player and widgets · **C** root, release and site.

| ID | Needs | Requested by | Owner | Blocking phase | Proposed shape | Status | Resolved in |
|---|---|---|---|---|---|---|---|
| G1 | `LaunchOptions.ramdisk` / `extra_args` so a device can boot a patched ramdisk (and pass `-prop`, R5) | C | A | 9d | `LaunchOptions { ramdisk, extra_args, cold_boot, wipe }` in `jidr-backend` | Done: also persisted per device as `Device.launch` (`ramdisk`, `extra_args`, `force_cold_boot`) | Phase 3b (ccbd9f0), 4a (02aa6d6) |
| G2 | ~~Bake Magisk into an image at creation~~ | C | A | — | — | Won't do: it duplicates Phase 9d and would bundle GPL Magisk (plan constraint 2) | — |
| G3 | A rooted on-device helper channel so widgets can run privileged actions (R1, R2, R4, R6, R9) | B | A | first 🟠 widget (post-MVP) | one on-device `jidr-helper` with a typed RPC the daemon proxies (O2) | Open (post-MVP) | |
| G4 | Confirm the guest kernel's cgroup version, network interface names and block devices on our target images (R2, R6) | A | A | before R2/R6 build | an "image facts" table in `docs/research/` per API level | In progress: API 31 done | `docs/research/2026-10-13-image-facts-api31-arm64.md` |
| G5 | Generated gRPC stubs from pinned AOSP emulator protos | B | A | before 7c | SDK proto + vendored fallback in `jidr-proto` | Done | Phase 3a (a90110d) |
| G6 | `jidr-widgets` exports `pub fn provider() -> Arc<dyn jidr_core::provider::CommandProvider>` so `jidr daemon` can register it | A | B | none (A wires it when it lands) | contract in `crates/jidr-core/src/provider.rs` | Done: `jidr daemon` registers `jidr_widgets::provider()` (`build_daemon` in `apps/jidr`) | Phase 8a (f206f6d); `feat(jidr): register the widgets provider` |
| G7 | `jidr-root` exports the same `pub fn provider()`; it uses `CommandProvider::on_booted` for `adb root` after each boot, and `Device.launch` (`ramdisk`, `force_cold_boot`) + `DaemonContext::stop_device/start_device` for Magisk | A | C | none (A wires it when it lands) | as G6 | Done: `jidr daemon` registers `jidr_root::provider()` with the widgets one (`build_daemon` in `apps/jidr`) | Phase 9c (a1eb985); merge of main into stream/c-root |
| G8 | `root.set` accepts an optional `magisk_sha256`: an APK already in the download cache (`jidr root set <dev> magisk --magisk <apk>` imports it via `cache.import` first, D30) | A | C | none | `jidr_sdk::DownloadCache::new(paths.downloads())`: `get_verified(sha)` offline, `fetch(url, sha, …)` for the pinned release | Done | Phase 9d |
| G9 | FYI: `jidr-cpio` adds `lz4` 1.28 (bundled liblz4), `flate2` 1.1 (`zlib` backend) and `liblzma` 0.4 (`static`) in its own manifest, so `Cargo.lock` changes on Stream C's branch. Regenerate the lockfile when merging; promote them to `[workspace.dependencies]` only if another crate needs them. | C | A | none | — | Open (info) | |
| G10 | `snapshot.save` records the device's root tag (`Device.root.mode` and `magisk_version` into `jidr_core::Snapshot`), and `snapshot.load` refuses a snapshot taken under a different root mode with `usage`, naming both modes (D29). `root.set` already sets `launch.force_cold_boot`, so the quick-boot snapshot is never resumed across a root change. | C | B | 10b (not blocking 9c) | tag at save time from the stored `Device`; compare on load | Open | |
| G11 | CI `fmt, clippy, tests (ubuntu-latest)` fails: `spikes/display/build.rs` reads `emulator_controller.proto` from `$ANDROID_SDK_ROOT/emulator/lib`, but the runner's preinstalled SDK has no emulator. Fall back to the vendored `crates/jidr-proto/proto/` (or reuse `jidr-proto`). Seen on PR #1, run 38526293836. | C | A | merging PR #1 | vendored-proto fallback in the spike's build script | Done | stream/a-core fb8d18e |
| G12 | CI emulator jobs fail to build `yeslogic-fontconfig-sys`: `--workspace --features emulator` builds the Slint GUI crates, but the emulator job doesn't install `libfontconfig1-dev libxkbcommon-dev` like the check job does. | C | A | merging PR #1 | copy the "Linux GUI build deps" step into the emulator job (or exclude `jidr-ui`/`jidr-gui` there) | Done | stream/a-core fb8d18e |
| G13 | `docs/design/tokens.json` has no Arabic sizes for the landing type roles (`type-web-hero`, `-hero-sm`, `-display`, `-headline`, `-lede`, `-body`): the Arabic group only covers the app roles. `jidr.css` has them (e.g. web hero 52/80) but they came from the design tool, not the tokens. The Arabic landing page (11b) needs them. | C | Abdurahman (design) | 11b | add `type-web-*-ar` styles to the "Arabic (RTL)" group | Done: six `type-web-*-ar` styles in the Arabic group, at the sizes the approved Landing-AR screens use (`jidr.css`); the site's bridge is gone and a test keeps `tokens.json` and `jidr.css` in step | `fix(design): Arabic landing type sizes in tokens.json (G13)` |
| G14 | Surface the pristine `ramdisk.img` SHA-256 (filled by the root engine in 9d) in `image.list` / `device.details`. The value is computed when patching and stored in the patched ramdisk's `.json` provenance sidecar today; `jidr-core::SystemImage.pristine_ramdisk_sha256` exists but is never populated by `image_catalog`. | C | A | 9e (root UX) | fill `pristine_ramdisk_sha256` in `jidr_sdk::image_catalog` (hash `ramdisk.img` per installed image) | Open | |
| G15 | D22 "auto-grant all": `RootState.auto_grant_all` is stored but no command reads or writes it. The 9e dev-tools toggle needs `root.auto_grant {device, enabled}` that sets Magisk's automatic response for app su requests (grant, or back to prompt) on a Full-root device and stores the flag; refused with `usage` on other modes. | C (9e UI) | C (`jidr-root`) | 9e | `root.auto_grant {device, enabled: bool} -> {device, auto_grant_all}`; protocol changelog row + regenerated command reference (it has no `root.*` entries yet) | Done: `root.auto_grant` writes Magisk's `su_auto_response` SharedPreferences value ("2" allow / "0" prompt) over adb root, stores `auto_grant_all`, applies live or on the next boot, and resets on leaving Magisk. The command reference still has no `root.*` entries because `jidr-daemon`'s `protocol_doc` test does not register the root provider — filed as G16 | `feat(root): root.auto_grant (G15)` |
| G16 | `crates/jidr-daemon/tests/protocol_doc.rs` builds the daemon with only the core + jobs providers, so the generated command reference in `docs/control-protocol.md` lists no `root.*` (or widget) commands even though `apps/jidr` registers them at runtime (G6/G7). Register `jidr_root::provider()` (and `jidr_widgets::provider()`) in that test so the reference documents every command. Needs `jidr-root`/`jidr-widgets` as dev-dependencies of `jidr-daemon` (a dev-dependency cycle with `jidr-root`). | C | A | before release | register the providers in `protocol_doc.rs` and regenerate | Open | |
| B2 | FYI, lockfile: `jidr-ui` adds `slint-build 1.18.1` (build) and turns on slint's `unstable-winit-030` feature (window events for file drops, transparent title bar) in its own manifest, so `Cargo.lock` changed on stream B. Regenerate on merge. | B | A | none | — | Open | |
| B3 | FYI, root UI slots for 9e: B ships a basic `RootBadge` (`crates/jidr-ui/ui/components/status.slint`) and a three-card root-mode picker in the create sheet so 7b works. C can restyle or replace them from `crates/jidr-ui/src/root/**`; ask B for a new slot through this table. | B | C | 9e | — | Done: `RootModeSelector` from `src/root` replaces `RootCards` (removed) in the create/edit sheet, Settings and Setup | 9e (B8) |
| B4 | Daemon-side settings the GUI's Settings › General shows: default root mode (also used by `jidr device create` without `--root`) and "save quick boot on stop" (D29). Today the GUI keeps both in `gui-prefs.json`, so the CLI doesn't see them. | B | A | none (7b ships with local prefs) | `settings.get` / `settings.set {default_root_mode, quick_boot}` | Open | |
| B5 | Bytes in job progress: `image.install` / `sdk.install` report only a fraction and `"{phase} {package}"`. The design shows "1.4 / 2.1 GB"; the GUI derives it from the catalog size × fraction. Real `done`/`total` bytes in `job.progress` data would make it exact. | B | A | none | `job.progress {…, bytes_done, bytes_total}` | Open | |
| B6 | The player's zip drop on a Full-root device calls `root.module.install {device, path}` (9d, D35) as a job. Until that command exists the player copies the zip to `/sdcard/Download` and says so. | B | C | none (fallback in place) | `root.module.install` as planned in 9d | Done: 9d ships `root.module.install` (`crates/jidr-root/src/magisk/mod.rs`); the player already calls it | Phase 9d |
| B7 | FYI for 9c/9d: snapshot root-mode tags live in `<avd>/jidr/snapshots.json` (`{id: {label?, root_mode, magisk_version?, created_at}}`), written by `snapshot.save` with the device's `root.mode` at save time. The id is the folder name Jidr picks (`Clean install` → `Clean_install`; the typed name is kept as `label`); commands accept the typed name. `snapshot.load` refuses a tag that differs from the device's current mode (D29) with `data.refused = "root_mode"`. The refusal dialog's "Set up Full root…" calls `root.set {device, mode}`. | B | C | none | — | Open | |
| B8 | Root UX slots for 9e (B3): (1) `src/lib.rs` `pub mod root;`; (2) `src/app.rs`: `crate::root::wire(&ctx)` in `run`, `crate::root::sync(self)` at the end of `Ctx::rerender`, `crate::root::golden_cases()` appended in `goldens::cases()` (and `goldens::shot`/`model` made `pub(crate)`); (3) `ui/app.slint`: import and export `RootUi`, place `RootLayer` as the top overlay under the title bar, Escape sets `RootUi.open = false`; (4) `widgets/panels.slint` `MorePanel`: `RootDevTools {}` first in Developer tools; (5) `RootModeSelector` replaces `RootCards` in `device_form.slint` (also shown when editing), `settings.slint`, `wizard.slint`; (6) `launcher::submit`: after `device.create` with Full root call `crate::root::start(c, id, RootMode::Magisk)` instead of `device.start`, and after `device.edit` with a changed mode call `crate::root::start(c, id, mode)`; (7) `ui/icons/shield-alert.svg` (Lucide); (8) C may write `tests/golden/root-*.png` | C (9e) | B | 9e | the API in `crates/jidr-ui/src/root/` (`wire`, `sync`, `start`, `golden_cases`, `RootUi`, `RootLayer`, `RootDevTools`, `RootModeSelector`) | Done: all slots wired; the edit sheet shows the root cards too (`DeviceForm.play-store` keeps Shell disabled on Play Store devices); Play Store cards are 134 px high. C still has to let `ModeCard`'s note use the taller card (its text box is fixed at 32 px). C may write `tests/golden/root-*.png`. | 9e (B8) |

**Status:** Open → Planned → In progress → Done (link the commit) / Won't do (with reason).

Rules:

- One row per need. Say *what* you need and *why*, not how the owner must build it. "Proposed shape" is a suggestion only.
- The owning stream checks this table at the start of every phase and serves anything that blocks another stream before starting new work.
- Never edit another stream's files to unblock yourself.

---

## Part C: Design opportunities

These are our own design choices, not parity items, surfaced by the research. IDs use **O#** to avoid clashing with the plan's Decisions log (D1–D34).

| ID | Opportunity | Why it's better | Related | Where it lives |
|---|---|---|---|---|
| O1 | Lean on **Magisk** rather than building our own su manager | MagiskSU already provides the allow/deny prompt and the policy store, and it's better maintained than anything we'd write. We download it at runtime and show its state. | R11, R12 | Already plan policy (Phase 9d, D22) |
| O2 | **One unified rooted helper**, not a privileged daemon per feature | A single `jidr-helper` with a typed RPC proxied by the daemon is easier to build, test, secure and audit than many privileged processes, and it gives one place to gate dangerous actions | R1, R2, R4, R6, R9 | G3 (post-MVP) |
| O3 | Use the **public gRPC protos** for display and control | Maintained by Google; we generate stubs from pinned AOSP protos instead of inventing our own protocol to the emulator | R15 | Plan (Phase 3a, ADR-0001) |
| O4 | ~~Prefer cgroup v2 `io.max` for disk throttling~~ | Disproven: the emulator kernel has no block throttling in either cgroup version (proof run) | R6 | Withdrawn; R6 needs a host-side route |
| O5 | **Scope the fuzzy features down** | Treating "flash a dropped zip" as Magisk modules only, and dropping Device Link, is clearer than chasing vague legacy behaviour | R10, R12 | R12 decision pending; R10 = Drop |
| O6 | **CLI- and MCP-first** control surface | — | all | Already in the plan (architecture; D5, D10) |

Note: matching these features gives device-identity spoofing, not a Play Integrity or SafetyNet bypass. That's a separate, moving target and out of scope.

---

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | created | Template with R1–R15 from the plan's parity map |
| 2026-10-13 | researched | Filled R1–R15 from public sources (AOSP emulator docs and protos, Genymotion public docs, Magisk docs) and set all to Researched. Added Part B rows G2–G5. The proofs are proposed and not yet run; no live emulator this pass. |
| 2026-10-13 | design opportunities | Added Part C consolidating Jidr-specific design choices |
| 2026-10-13 | review vs plan | Added the plan constraints. R5 (no image edits; `-prop` / Magisk `resetprop` routes; confidence → Medium), R7 (host virtual webcam instead of a custom HAL), R11 (covered by 9d, never bundle Magisk), R13/R14 (covered by 7c), R15 (fallback only, RTC proto not in the local SDK, token-auth proof). G1/G5 → Planned, G2 → Won't do. Part C renamed D# → O#. Marked secondary sources. |
| 2026-10-13 | decisions | D35–D38: R12 zip → Magisk module; R3/R7/R8 native parts into Phase 8a; findings Accepted (R10 Rejected). |
| 2026-10-13 | proof run | Ran the proofs on an API 31 arm64 AVD. R2/R3/R7/R8 native proven; R5 `-prop` disproven; R6 guest throttle impossible → Post-MVP; R1 operator property doesn't reach telephony; R15 `streamScreenshot` ≈30 fps at window size. G4 partly done. |
