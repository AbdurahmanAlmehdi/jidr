# ADR-0002: Full root on Play Store images (Phase 9a)

- **Status:** Proposed. Waiting for Abdurahman's decision; Phase 9a is not Done.
- **Stream:** C
- **Date:** 2026-10-13

## Context

D6 and D21 ask for full Magisk support on `google_apis_playstore` images, trying in order: (1) a boot-time grant of the adb shell's uid in Magisk's policy store, (2) an automated tap on Magisk's Grant prompt, (3) a guided tap as a beta fallback.

Play Store images don't allow `adb root`. So the shell can't write Magisk's policy itself the way it can on `google_apis` images, and the first `su` from the shell shows Magisk's prompt on the device.

## What the agent did and didn't do

- Options (1) and (2) were **not attempted** by the Stream C agent. Both grant root to the shell without a person approving Magisk's prompt. An earlier attempt at this phase was stopped by a safety check, so the agent left these two options to Abdurahman rather than rebuild them.
- Option (3) keeps the person in the loop. It is specified below so 9d can ship it as the Play Store beta (D6 fallback).

## Proposed beta flow: guided tap (option 3)

1. Full-root setup runs exactly as on `google_apis` (9d): patched private ramdisk, cold boot, Magisk app install, environment setup, one reboot.
2. Jidr opens the device window and shows one instruction beside the screen: "Magisk will ask to grant root to Shell. Tap Grant." Magisk's own "Forever" option is left to the user.
3. Jidr runs `su -c id` from the adb shell, which raises Magisk's normal prompt. It waits up to 60 s, with a Cancel button.
4. Verify: `adb shell su -c id` → `uid=0` and `su -c 'magisk -v'`. The badge reads "Rooted ✓ · Beta (Play Store image)".
5. Denied or timed out: the badge reads "Root unverified", with "Try again" (repeats step 3) and "Revert" (9d revert).
6. Headless or CLI (`jidr root set magisk` on a Play Store image): the command prints the same instruction and waits; `--json` reports `{"state": "awaiting_user_grant"}` until verified. There is no headless bypass.

Apps keep Magisk's normal per-app prompt (D22). The "auto-grant all" toggle stays opt-in and off.

## Decision needed

Abdurahman: ship option (3) as the Play Store beta for the MVP, or do options (1)/(2) yourself and record the result here.

## Consequences

- With (3), Play Store full root needs one tap per device, once. `google_apis` images stay zero-touch.
- The acceptance criterion's "beta pass" also asks this ADR to say why (1) and (2) failed. They weren't tried, so 9a stays open until Abdurahman decides.
