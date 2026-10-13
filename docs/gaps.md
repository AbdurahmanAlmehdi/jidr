# Gaps

Two tables. Owners check this file at the start of every phase.

## Cross-stream requests

A stream that needs a hook, command or change in another stream's files adds a row and keeps working on something else. The owning stream marks it Done with the commit that delivers it.

| # | Needs | From stream | Blocking phase | Owner | Status |
|---|---|---|---|---|---|
| C1 | FYI: `jidr-cpio` adds `lz4` 1.28 (bundled liblz4), `flate2` 1.1 with the `zlib` backend and `liblzma` 0.4 (`static`) in its own manifest, so `Cargo.lock` changes on Stream C's branch; regenerate the lockfile on merge. Promote them to `[workspace.dependencies]` only if another crate needs them. | C | none | A | Info |

## 🔎 Research gaps (D11)

Parity-map features that need Abdurahman's research before any agent works on them. Out of MVP scope until a finding lands here.

| Feature | Question | Finding | Status |
|---|---|---|---|
| Baseband operator & SIM identity | How does Genymotion set operator/SIM MCC/MNC, operator name, MSIN? Does the AOSP modem simulator expose it? | | Open |
| Network packet loss / bad DNS profiles | How does Genymotion inject loss and DNS failure? The AOSP console has only speed/delay. | | Open |
| Wi-Fi signal strength | How to set Wi-Fi signal strength on the AOSP emulator? | | Open |
| IMEI/MEID | How does Genymotion change IMEI live? | | Open |
| Build props (MODEL, BRAND, FINGERPRINT) | `-prop` only covers `qemu.*`; resetprop needs Magisk; ramdisk/vendor overlay? | | Open |
| Disk I/O rate limit | Genymotion's mechanism; the AOSP emulator doesn't expose QEMU block throttling | | Open |
| Camera image/video injection | virtualscene posters? a v4l2loopback equivalent on macOS? | | Open |
| Biometrics scenarios | dirty / partial / too-fast fingerprint scenarios | | Open |
| Gamepad passthrough | Genymotion's mechanism; `sendKey` gamepad keycodes + `gilrs`? | | Open (post-MVP) |
| Device Link | real phone mirrors sensors | | Open (post-MVP) |
| Superuser prompt app | which su app Genymotion ships (adb-mode parity only) | | Open |
| Flash semantics | what Genymotion's zip flash does exactly | | Open |
