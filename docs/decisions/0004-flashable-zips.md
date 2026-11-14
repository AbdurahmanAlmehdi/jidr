# ADR-0004: Flashable zips on Jidr devices

- **Status:** Proposed (2026-11-14)
- **Issue:** #72 (parity row "Zip flash"). Builds on D35 and D3.
- **Built in the same pull request:** zip-type detection, the `file.flash` command, `jidr flash`, the player's drop routing, and an emulator test of a module install.

## Context

Genymotion's free tier flashes a zip dropped on the device. Jidr does part of that today (D35): a dropped zip installs as a Magisk module on a Full-root device and is copied to `/sdcard/Download` on any other device. Nothing looks inside the zip, so a TWRP-style zip on a Full-root device fails deep inside Magisk with "This zip is not a Magisk module!", and `jidr flash` only says it isn't available yet.

Plan constraint 1 (D3, D35) limits what Jidr can do: Jidr never bundles anything, and it changes nothing of a device's system image except its own per-device copy of the ramdisk. The emulator has no recovery, no bootloader and no fastboot: it boots the SDK package's read-only partition images directly.

What people flash on phones, and whether each one fits:

| Zip | What's in it | On a Jidr device |
|---|---|---|
| Magisk module | `module.prop` at the zip's root, optional `customize.sh`, `system/`, `service.sh`, `post-fs-data.sh`, `system.prop`, `sepolicy.rule`; often also `META-INF/com/google/android/update-binary` so recovery can flash it | Works through `magisk --install-module` (D35). This ADR adds a check before the install and an emulator test. |
| Zygisk module (LSPosed, Shamiko, Play Integrity fixes…) | A Magisk module plus `zygisk/<abi>.so` | Works the same way. Magisk turns Zygisk **on by default on emulators** (its `zygisk` setting defaults to 1 when `ro.kernel.qemu` or `ro.boot.qemu` is 1), so no extra step is needed. |
| Riru module | A Magisk module plus `riru/` | Doesn't work: Magisk skips Riru modules while Zygisk is on, and Riru itself is discontinued. |
| GApps (OpenGApps, MindTheGapps, NikGapps) | Recovery zip: `META-INF/com/google/android/update-binary`, no `module.prop`; writes `/system` | Not needed: `google_apis` images ship Play services, and `google_apis_playstore` images also ship the Play Store. Flashing them would mean writing `/system`, which needs recovery. |
| ARM translation (libhoudini, libndk_translation) | Recovery zip for x86 images | Irrelevant: Jidr runs arm64 images on Apple Silicon (D15), so ARM apps run natively. |
| Other recovery zips (TWRP flashables, custom kernels, Xposed for Lollipop, debloaters) | `update-binary` that mounts and writes partitions from recovery | No recovery exists, and running the script in Android would write the system image or partitions, which plan constraint 1 rules out. |
| OTA and factory images | `payload.bin`, `META-INF/com/android/metadata`, or partition `.img` files with `android-info.txt` | No fastboot or partition flashing on the emulator: the partitions are the SDK image's files, shared read-only by every device that uses that image (D3). To change Android versions, create a device from another image. |

## Decision

1. **Detect the zip type from its file list, before anything touches the device.** Jidr reads the zip's central directory (it never extracts or runs anything on the Mac) and sorts it into one of these:
   - `magisk_module`: `module.prop` at the root with a valid `id`;
   - `zygisk_module`: the same, plus a `zygisk/` folder;
   - `riru_module`: the same, plus a `riru/` folder, or `id=riru-core`;
   - `recovery`: `META-INF/com/google/android/update-binary` and no root `module.prop`;
   - `image`: `payload.bin`, `META-INF/com/android/metadata`, or `android-info.txt` with partition `.img` files;
   - `other`: anything else, including zips that aren't zips.

   A module that is a module zip with one problem gets a typed `usage` error that names the fix instead of Magisk's generic one:
   - `module.prop` inside a single top-level folder (a zipped folder, often a GitHub "Download ZIP"): zip the folder's contents instead;
   - a missing or invalid `id` (Magisk's documented rule: a letter, then at least two letters, digits, `.`, `_` or `-`). Magisk itself doesn't check the id and uses it as a directory name;
   - an id starting with `jidr_`, which is reserved for Jidr's own modules (`jidr_cacerts`).

2. **One command for every zip: `file.flash {device, path}`** (root provider; long-running, locks the device, destructive for MCP). The CLI's `jidr flash <device> <zip>` and the player's drop call it. It routes by type:

   | Type | Full-root (`magisk`) device | Any other device |
   |---|---|---|
   | `magisk_module`, `zygisk_module` | Install through `magisk --install-module`, cold boot, then check that Magisk loaded the module (`/data/adb/modules/<id>` without `disable` or `remove`) and root still verifies | Copy to `/sdcard/Download` with a note that modules need Full root (D35, unchanged) |
   | `riru_module` | Refused (`usage`): Magisk skips Riru modules while Zygisk is on; use the module's Zygisk build | Same refusal |
   | `recovery` | Refused (`usage`): the emulator has no recovery, and the zip would write the system image | Same refusal |
   | `image` | Refused (`usage`): no fastboot or partition flashing; create a device from another system image | Same refusal |
   | `other` | Copy to `/sdcard/Download` | Copy to `/sdcard/Download` |

   The result says what happened: `{device, kind, action: "module_installed" | "pushed", module?: {id, name, version, zygisk}, remote?, note?, zygisk_enabled?, verification?}`. Refusals carry `error.data.kind`. `jidr push` still copies any file, so a refused zip can still reach the device on purpose.

3. **No Zygisk switch in Jidr.** Magisk already enables Zygisk on emulators, and the Magisk app (installed on every Full-root device) has the switch in its settings. When a Zygisk module is installed while Magisk's `zygisk` setting is 0, `file.flash` installs it anyway and its result says the module won't load until Zygisk is turned on in the Magisk app. Revisit if people ask for it in Jidr.

4. **`root.module.install` stays** as the explicit "install this as a module" command, with the same pre-checks. `file.flash` is the one the drop, the CLI and agents use.

5. **No image flashing**, for the reasons in the table. The parity row "Zip flash" becomes ✅ for modules, with refusals for the rest stated in [root.md](../root.md#flashable-zips).

## Consequences

- A zip dropped on the player behaves the same way as `jidr flash`, since both are the same protocol command. The GUI no longer decides from the file name alone.
- D35 changes in one way: a recovery, Riru or image zip dropped on any device is refused with an explanation instead of being copied to Download. Module zips on a device without Full root are still copied, as D35 says.
- Zygisk modules work without a Jidr setting. If Magisk ever changes its emulator default, the note in the result says what to do.
- The emulator test (`crates/jidr-root/tests/module_emulator.rs`) installs a small Apache-2.0 test module written for Jidr from Magisk's public module guide, checks its `service.sh` marker and its `system.prop` property after the reboot, then removes it and checks both are gone after the next reboot.
- Open for later: a module list and remove command (`root.module.list` / `root.module.remove`); a Zygisk switch if users ask.
