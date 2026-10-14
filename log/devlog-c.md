# Stream C devlog (Root, ship & site)

Branch `claude/trusting-grothendieck-401ea9` (fast-forwarded onto `stream/a-core` Phase 0, `6ae5ad6`), `CARGO_TARGET_DIR=target/agent-c`.

Environment notes (2026-10-13):
- Rust 1.99 via Homebrew `rustup` (keg-only, `/opt/homebrew/opt/rustup/bin` on `PATH`); `cargo-nextest`, `protoc`, `lz4` 1.10.0, `xz` from Homebrew; Node 20.20 / npm 10.8.
- Arm64 system images that are **actually** present in the Studio SDK (read-only): API 25 `google_apis`, API 31 `google_apis`, API 36 `google_apis_playstore_ps16k`. The API 34 `default`, API 36 `google_apis` and Baklava Play Store directories contain only an `.installer` folder (interrupted installs), so they can't boot. API 35 is x86_64 only.
- The API 36 Play Store `ramdisk.img` sits next to a `ramdisk.img.backup` left by an earlier rootAVD run. Both hash to `90f2f636…b3a1`, and the cpio has no Magisk files, so the image is pristine. Jidr never writes there (D3, D7).

Order: 9b before 9a, because the 9a spike patches the ramdisk on the host and uses `jidr-cpio` to do it. 11a is independent.

---

## Phase 9b: Ramdisk cpio library — plan

What the real images look like (measured on the five local `ramdisk.img` files):
- API 29+ emulator ramdisks are **several lz4-legacy streams concatenated**, each with its own `02 21 4c 18` magic: a generic ramdisk (`init`, mount points) and a vendor fragment (`first_stage_ramdisk/fstab.ranchu`, modules). API 25 is a single gzip member.
- Each stream holds one newc (`070701`) archive. mkbootfs quirks: inodes start at 300000, the `TRAILER!!!` entry has `mode 0755` and a real inode, `dev` appears twice, and zero padding follows the trailer to a 256-byte boundary.
- Re-compression is byte-identical with the reference implementations: liblz4 HC level 12 per 8 MiB block (`lz4 -l -12`), and zlib deflate level 6 with a raw 10-byte header (`gzip -n -6`). miniz_oxide and zlib-rs do **not** reproduce zlib's bytes, so `jidr-cpio` uses the `lz4` crate (bundled C liblz4, MIT/BSD-2) and `flate2` with the `zlib` backend. xz goes through `liblzma` (bundled, MIT/Apache + 0BSD). All permissive and Slint-free.

API:
1. `cpio::Archive` / `Entry`: parse and write newc (`070701`) and newc-crc (`070702`). Every header field, the trailer entry and the bytes after the trailer are kept, so an unmodified archive writes back byte-identical. Edits: `get`, `get_mut`, `contains`, `insert` (replace in place or append before the trailer), `remove`, `rename`, constructors for files, directories and symlinks; new entries get the next free inode.
2. `compress::Compression` { `None`, `Gzip { level, header }`, `Lz4Legacy { level }`, `Xz { preset, check }` }: detect by magic, decompress one stream and report how many input bytes it consumed, compress with the same settings.
3. `Ramdisk`: splits a `ramdisk.img` into its concatenated streams (`Segment { compression, archive }`), keeps any trailing bytes, writes it back.
4. Typed `thiserror` errors with byte offsets; no `unwrap`/`expect`/`panic` outside tests; decompression bounded (8 MiB legacy blocks).

Tests:
- Unit tests per module.
- Property tests (proptest): arbitrary archives round-trip through write → parse; every compression round-trips arbitrary bytes; edits keep the archive parseable.
- Synthetic fixtures (our own content, so they can be committed) compressed by the reference CLIs `lz4 -l -12`, `gzip -n -6` and `xz -9 --check=crc32 -T1`, with the script that made them; the round-trip must be byte-identical.
- Real images: every `ramdisk.img` found under `$JIDR_TEST_SDK/system-images` (default Studio SDK, read-only) round-trips byte-identically through decompress → parse → write → compress. The test prints and passes when no SDK is present (CI check job), so the fixtures carry the guarantee there.

## Phase 9b: Ramdisk cpio library — done (2026-10-13)

Built (`crates/jidr-cpio`, Apache-2.0, no GPL code read or copied):
- `cpio::{Archive, Entry, Format}`: newc `070701` and `070702` read/write. Every header field, the trailer entry and the bytes after it are kept. Edits: `get`/`get_mut` (last duplicate wins, as on unpack), `insert` (replace in place keeping the inode, or append with the next free inode), `remove` (all duplicates), `rename`, `Entry::{file, dir, symlink}`.
- `compress::{Compression, decompress, XzCheck}`: detection by magic; one stream decoded at a time with its consumed length; gzip keeps its original header; lz4 legacy in 8 MiB blocks with liblz4 HC 12; xz via liblzma with the header's check. Decompression is capped at 1 GiB, xz memlimit 512 MiB.
- `Ramdisk { segments, trailing }`: splits concatenated streams (the emulator's generic + vendor fragments), keeps zero padding, `get` resolves the effective entry across segments.

Validation: `cargo nextest run -p jidr-cpio` → 31 passed (unit, property and round-trip tests). Every `ramdisk.img` in the local SDK round-trips **byte-identically** through decompress → parse → write → compress: API 25 arm64 (gzip, 1 segment), API 31 arm64 and x86_64, API 35 x86_64 `google_apis` and `google_apis_playstore`, API 36 arm64 Play Store ps16k (lz4 legacy, 2 segments each). The committed fixtures (made by bsdcpio, `lz4 -l -12`, `gzip -n -6`, `xz -9 --check=crc32 -T1`; `tests/fixtures/make.sh`) carry the same guarantee on CI. `cargo xtask ci` passes.

Notes and open issues:
- C dependencies on purpose: the `lz4` crate (bundled liblz4 1.10), `flate2` on system zlib (`libz-sys`) and `liblzma` (bundled, `static`). Pure-Rust deflate/lz4 don't reproduce the SDK bytes. All MIT/Apache/BSD/Zlib; license-check passes. Filed gap C1 (FYI, `Cargo.lock` changes).
- A property test found a real bug before commit: 1–3 bytes of zero padding after an lz4 stream were reported as a truncated block. Fixed, with a regression test.
- Limitation, documented in code: an lz4 legacy frame has no end marker, so it can only be followed by another lz4 frame (as AOSP does). The kernel's own unpacker has the same rule.
- xz preset and lz4 level aren't stored in the streams, so they default to Android's (`-9` crc32, `-l -12`). Uppercase hex headers parse but are written lowercase (mkbootfs and libarchive both write lowercase).

---

## Phase 11a: Coming-soon site — plan

`docs/design/` has only PRODUCT.md (no brand kit, tokens or landing designs yet), so this is the token-driven placeholder the brief asks for.

1. `site/`: Astro 5.18 + `@astrojs/vue` 5.1 (Astro 7 needs Node ≥ 22.12; the dev shell and many CI images are on Node 20), static output for Cloudflare Pages (D16). Not deployed: deploying and domains wait for Abdurahman.
2. Tokens: `src/styles/tokens.css` holds every colour, type, space and radius value as `--jidr-*` custom properties (named to map 1:1 onto the future `tokens.json` / Slint `Theme`), light and dark. Components use tokens only, so restyling is one file.
3. Content (D34): pitch, root showcase, "Genymotion's paid features, free" grid (text only, no logos), the honest Play Integrity note, Star on GitHub, Sponsors link, trademark notice. No email capture, no analytics (D17), no web fonts, no tracking.
4. One Vue island (`client:visible`): the root-mode picker in the showcase (None / Shell root / Full root, with what each does). Everything else is static HTML.
5. Links live in `src/config.ts` (`jidr-dev/jidr`, `github.com/sponsors/jidr-dev`) because the org and Sponsors don't exist yet (D26: nothing created without asking).
6. Validation: `npm ci && npm run build`, then Lighthouse CI on `dist/` (performance and accessibility ≥ 95).

## Phase 11a: Coming-soon site — done (2026-10-13)

Built (`site/`, Astro 5.18.2 + `@astrojs/vue` 5.1.4 + Vue 3.5.44, exact pins, `package-lock.json` committed):
- Sections: header (wordmark "Jidr جذر", Star on GitHub), hero (pitch + a "planned CLI" terminal preview using the PRODUCT.md §7 snippet), root showcase, "Genymotion's paid features, free" (text only; 7 paid-tier features in the MVP plan + 2 new in Jidr), "How it works, and what it won't do" (SDK downloaded under the user's license, Magisk never bundled, Studio untouched, no telemetry), footer (licences, Sponsors link, trademark and non-affiliation notice).
- The Play Integrity note is on the page, next to the root picker.
- One Vue island (`RootModes.vue`, `client:visible`, native radio inputs): None / Shell root / Full root, with steps, the verify command and supported images. Everything else is static. The page ships ~31 kB of gzipped JS.
- Tokens: `src/styles/tokens.css` (light + dark, every text pair ≥ 5.4:1, listed in the file), system fonts only, `prefers-reduced-motion` respected, skip link, visible focus rings, no horizontal scroll at 375 px.

Validation: `cd site && npm ci && npm run build && npx @lhci/cli autorun --collect.staticDistDir=dist --assert.preset=lighthouse:recommended` → all assertions pass; three runs scored performance 100, accessibility 100, best practices 100, SEO 100. Checked by eye in the browser pane in dark (desktop) and light (375 px); the island hydrates and switches modes.

Notes and open issues:
- Lighthouse caught one issue that I fixed: the wordmark link's `aria-label` didn't match its visible text.
- Astro 7 needs Node ≥ 22.12; the default shell here is Node 20.20, so the site uses Astro 5 (`engines: >=20.3`). Moving to Astro 7 later is a version bump plus Node 22 on Cloudflare Pages.
- Placeholders waiting on Abdurahman: the brand kit and design tokens (D34; `docs/design/` has only PRODUCT.md), the favicon (a plain root-into-phone stroke, not a logo), and the links in `src/config.ts` (`github.com/jidr-dev/jidr`, `github.com/sponsors/jidr-dev`: the org, repo and Sponsors profile don't exist yet, D26).
- Not deployed (needs Abdurahman's go-ahead, D16/D34).

## Phase 11a: brand assets (D39) — done (2026-10-13)

After merging main (`c11cc06`: gaps proof run, D35–D40, brand kit), I replaced the placeholder favicon and logo:
- `site/public/` gets copies of `assets/brand/web/{favicon.svg, favicon.ico, favicon-32.png, apple-touch-icon.png, og-image.png, symbol.svg}`.
- The header shows `symbol.svg` through a CSS mask tinted with the accent token. `Base.astro` links the .ico, .svg and apple-touch icons and sets `og:image` / `twitter:image` (1200×630, `summary_large_image`).
- Accent tokens now use the brand blue (light `#1267E0`, dark `#2A88FA`), and the Arabic wordmark is lilac `#B9A6F7` on dark. A new `text-on-accent-subtle` token keeps the "New in Jidr" tags at ≥ 5.3:1. Every pair is listed in `tokens.css`.

Validation: `npm ci && npm run build && npx @lhci/cli autorun --collect.staticDistDir=dist --assert.preset=lighthouse:recommended` → all assertions pass; 100/100/100/100 on three runs. Checked by eye in light and dark.

## Phase 9a: Play Store root spike — blocked on a decision (2026-10-13)

- Options (1) and (2) of D21 grant root to the adb shell on Play Store images without a person approving Magisk's prompt. An earlier attempt at this phase in this session was stopped by a safety check, so I didn't build them; they're left to Abdurahman.
- `docs/decisions/0002-play-store-root.md` (Proposed) specifies option (3), the guided tap, as the Play Store beta, including the CLI/JSON behaviour. Plan status: Blocked.
- No spike code, no Magisk download, no images installed.

---

## Phase 9c: adb root mode — plan

Merged `protocol-v0.1` (`68eb049`). Stream A's own branch gets the lockfile from the tag; this branch adds `jidr-cpio`'s six crates on top (G6). Gaps A1/A2 from `stream/a-core` are carried into Part B as G7/G8.

`crates/jidr-root` (Apache-2.0, no Slint), `pub fn provider() -> Arc<dyn CommandProvider>` (G8):
1. `root.status {device}`: the stored `RootState` plus whether the image allows adb root. No lock.
2. `root.verify {device}`: on a running device, runs `adb shell id` and checks the uid against the mode (none → non-zero, adb → 0; magisk is 9d). Writes `health` + `verified_at`. A mismatch is `root_failed` (exit 7) after the failed health is stored. Locks the device.
3. `root.set {device, mode}`: `none | adb` (magisk → usage error until 9d). Refuses `adb` on Play Store images (D7, `ImageRef::allows_adb_root`). Persists the mode with health `unverified` and sets `launch.force_cold_boot` (D29: root changes force a cold boot; the snapshot taken under the old mode isn't reused). If the device is running, applies the mode live (`adb root` / `adb unroot`, wait for the device, verify) so the user doesn't wait for a reboot. Locks the device, long-running (job).
4. `on_booted`: adb-mode devices get `adb root` + verify after every boot; failures surface as `device.hook_failed` events and `health: failed`.
5. Snapshot root tags belong to `snapshot.*` (Stream B, 10b); filed as gap G9.

Tests:
- Offline: a `Daemon` with the fake backend, a fake Studio SDK and a stateful fake `adb` (remembers root/unroot and answers `shell id` accordingly). Covers set/verify/status, the boot hook, Play Store refusal, magisk-not-yet, unknown params.
- Emulator (`--features emulator`, `adb` filter): a real `AospBackend` daemon with the Studio SDK read-only and AVDs in a temp dir. Cycles none → adb (live) → cold boot (hook re-applies root) → none, verifying each step. Locally on API 31 `google_apis` arm64 (the only arm64 `google_apis` image installed; 33/35 run on Linux KVM CI, D28).

## Phase 9c: adb root mode — done (2026-10-13)

Built (`crates/jidr-root`, Apache-2.0; deps `jidr-core`, `jidr-adb`; dev-deps `jidr-daemon`, `jidr-backend`):
- `jidr_root::provider()` (G8) with `root.status` (no lock), `root.verify` (locks the device) and `root.set {device, mode}` (locks the device, long-running → job).
- `root.set`: refuses `adb` on images that don't allow it (D7) and `magisk` until 9d. Persists the mode with health `unverified`, sets `launch.force_cold_boot` on a real change (D29), and applies the change live on a running device (`adb root`/`unroot`, then polls `adb shell id` until adbd is back with the right uid, 60 s cap). A stopped device gets it at the next (cold) boot.
- `on_booted`: adb-mode devices are rooted and verified after every boot. A failure sets `health: failed`, and the daemon emits `device.hook_failed`.
- `root.verify`: `id` (or `su -c id` for magisk) → `{uid, ok, health, verified_at}`, stored on the device; a mismatch is `root_failed` (exit 7) carrying the result in `data`.

Validation:
- `cargo nextest run -p jidr-root --features emulator -- adb` → 5 passed. The real-emulator test (`adb_emulator.rs`, API 31 `google_apis` arm64, Studio SDK read-only, AVD in a temp dir): cold boot 24.6 s → `uid=2000` → live `adb root` 1.2 s → `uid=0` → stop, cold boot + hook 15.3 s → `uid=0`, health `ok` → live `adb unroot` → non-root → magisk refused → delete.
- Offline (`adb_mode.rs`, 9 tests): a daemon with the fake backend, a fake Studio SDK and a stateful fake adb. Covers the boot hook, live switches, the next-boot cold boot, the Play Store refusal, magisk refused, a lost root recorded as failed, and params validation.
- `cargo xtask ci` → 149 passed.

Open issues:
- API 30/33/35 (x86_64) are covered by the Linux KVM CI matrix, which can't run until the branch is pushed. The push was denied by this session's permission check; it waits for Abdurahman.
- Snapshot root tags are Stream B's `snapshot.*` (10b): filed as G9.
- Stream A wires `jidr_root::provider()` into `jidr daemon` (G8); the CLI's `root set/verify/status` already target these method names.

---

## Phase 12a: Unsigned release pipeline — plan

`cargo xtask release [--dry-run] [--scan] [--out <dir>]` in `xtask/src/release.rs`, the only xtask file Stream C owns. So no new xtask deps: hashing, signing and DMG work go through the system's `shasum`, `codesign`, `hdiutil` and `plutil`.
1. `cargo build --release --locked -p jidr -p jidr-gui` (host arm64 on the dev Mac and the `macos-15` runner).
2. `Jidr.app`: `Contents/MacOS/Jidr` (GUI), `Contents/Helpers/jidr` (CLI + daemon; not `MacOS/jidr`, which collides with `Jidr` on the case-insensitive APFS default), `Contents/Resources/Jidr.icns` (D39), the licences + NOTICE, and `Info.plist` from `packaging/macos/Info.plist.in` (bundle id `ly.jidr.Jidr`, `@VERSION@` from the workspace version). `plutil -lint`.
3. Ad-hoc signature (`codesign -s -`, inside-out), so the bundle is valid on Apple Silicon. Developer ID signing and notarization are 12b. Without the signing secrets, CI gets a `::warning::` (D32).
4. `Jidr-<version>-arm64.dmg` (UDZO, with an `/Applications` link), then mounted read-only to prove it mounts.
5. `--scan`: walks the bundle and the mounted DMG and fails on anything Jidr must never ship. Checks file names (emulator, qemu, adb, fastboot, sdkmanager/avdmanager, `*.img` system/vendor/ramdisk/userdata images, kernels, anything named magisk) and content magic (ELF, ext4, Android sparse and boot images, lz4-legacy/cpio ramdisks, dex, APKs). Pure classifier with unit tests.
6. `SHA256SUMS.txt` and `VERSION` next to the DMG.
7. `.github/workflows/release.yml`: on push to `release` (and manual dispatch), `macos-15` arm64 runner, `cargo xtask release --scan --out dist`, then `gh release create v<version> --draft` with the DMG and `SHA256SUMS.txt`. Nothing is published without a human promoting the draft.

## Phase 12a: Unsigned release pipeline — done (2026-10-13)

Built:
- `xtask/src/release.rs`: `cargo xtask release [--dry-run] [--scan] [--out DIR]` (release build → `Jidr.app` → ad-hoc signature → UDZO DMG → read-only mount check → optional scan → `SHA256SUMS.txt` + `VERSION`). On CI without signing secrets it prints `::warning::` (D32); a set identity also warns, since Developer ID signing is 12b.
- The scan rejects by name (SDK tools, qemu, kernels, system/vendor/ramdisk/userdata images, SDK package metadata, anything named magisk) and by content (ELF, Android boot/sparse/ext4 images, lz4-legacy/cpio ramdisks, dex, APKs). It runs on the bundle and on the mounted DMG and doesn't follow the `/Applications` link. 5 unit tests.
- `packaging/macos/Info.plist.in` (bundle id `ly.jidr.Jidr`, arm64, macOS 13+, developer-tools category) and `packaging/macos/README.md`.
- `.github/workflows/release.yml`: push to `release` (or manual dispatch) on `macos-15` → `cargo xtask release --scan --out dist` → `gh release create v<version> --draft` with the DMG and `SHA256SUMS.txt`. Drafts only: a human publishes.

Validation: `cargo xtask release --dry-run --scan` → exit 0; `Jidr-0.1.0-arm64.dmg` (0.8 MB), `shasum -c` OK. Mounted by hand: `Jidr.app/Contents/{MacOS/Jidr, Helpers/jidr, Resources/Jidr.icns, Resources/licenses}`; both binaries are Mach-O arm64 and run (`jidr-gui 0.1.0`, `jidr 0.1.0`); `codesign -dv` → ad-hoc, `ly.jidr.Jidr`; `open` launches the app (today's GUI is the Phase 0 stub, so it exits at once); `spctl` rejects it, the expected unsigned path. `cargo xtask ci` passes.

Open:
- Manual: right-click → Open on a downloaded (quarantined) DMG needs a human.
- `release.yml` is unvalidated until the branch is pushed. Its first real run needs a `release` branch, which Abdurahman creates.
- 12b (Developer ID, notarization, Homebrew cask) stays Blocked on the secrets.

## Phase 12b: Signing, notarization, Homebrew — prepared, still Blocked (2026-10-13)

- `packaging/macos/sign.sh app|dmg`: hardened runtime, timestamped, inside-out signing with `JIDR_SIGNING_IDENTITY`; the DMG is signed, notarized (`notarytool --wait`, app-specific password from secrets) and stapled.
- `packaging/macos/verify.sh <dmg>`: mounts read-only; checks the DMG signature, stapled ticket and `spctl --type open`, then the app's deep strict signature, hardened runtime and `spctl --type execute`. It always detaches.
- `packaging/homebrew/jidr.rb`: cask template (arm64, macOS 13+, `jidr` CLI linked from `Contents/Helpers`), for a future `jidr-dev/homebrew-tap`.
- Checked: `bash -n`, `ruby -c`; `sign.sh` refuses missing input or identity; `verify.sh` correctly fails on today's ad-hoc 12a DMG ("code object is not signed at all") and leaves nothing mounted.
- Not done (needs the secrets): wiring `sign.sh` into `cargo xtask release` and `release.yml`, then the real `verify.sh` pass and a `brew install --cask` from a tap. Status stays Blocked.

---

## Session summary (2026-10-13, end of this session)

Done: 9b (`jidr-cpio`), 9c (adb root mode), 11a (site + D39 brand), 12a (unsigned release pipeline). Merged main (`c11cc06`) and `protocol-v0.1` (`68eb049`).

Blocked on Abdurahman:
- **Push and draft PR:** this session's permission check denied `git push -u origin HEAD:stream/c-root`. Nothing has been pushed, so CI (including the Linux KVM API 30/33/35 root matrix and `release.yml`) hasn't run.
- **9a:** ADR-0002 (Proposed) specifies the guided-tap beta for Play Store images. The no-consent options (1) and (2) were not attempted.
- **9d (Magisk mode):** not started in this session. An earlier response here was stopped by a safety check while working on this area (Magisk patching and the automatic shell grant), so it waits for Abdurahman to review before an agent builds it. The pieces it relies on exist: `jidr-cpio` (byte-identical ramdisk patching), `Device.launch.ramdisk` / `force_cold_boot`, `jidr-root`'s verify (`su -c id`) and provider.
- **12b:** scripts prepared; needs the Developer ID and notarization secrets.

Blocked on other streams:
- **9e:** needs Stream B's `jidr-ui` shell and snapshot harness (7a); B hasn't started. 9e also needs 9d.
- **11b:** needs the 9d Magisk demo.
- Gaps: G8 (`jidr_root::provider()` ready for A to register), G9 (snapshot root tags, B 10b), G6 (lockfile FYI).

Open gaps and notes: API 31 is the only arm64 `google_apis` image installed here (API 34 `default` and API 36 `google_apis` folders are empty); the API 36 Play Store ramdisk hashes the same as its rootAVD `.backup` and has no Magisk files (checked with `jidr-cpio`), so it's pristine.

## Fix: emulator test cleanup (2026-10-14)

A peer session found 13 orphaned emulators (Stream A's `agent-phone`, `jidr_resilience`) filling the Mac's memory. Forwarded to Stream A. Stream C's own e2e (`crates/jidr-root/tests/adb_emulator.rs`) also only stopped its emulator on the happy path, so a red run would leak too. Fixed:
- `EmulatorGuard` (Drop) kills any `qemu-system` still running the test AVD: SIGTERM, then SIGKILL after 5 s.
- A backstop before boot kills a leftover from a run whose process was SIGKILLed (nextest timeout or Ctrl-C), since Drop never runs there.
- `JIDR_ROOT_E2E_FAIL_AFTER_BOOT=1` fails the test on purpose, to check the guard.

Checked (regression): a deliberately failing run leaves no qemu. A test process SIGKILLed after boot leaves an orphan (ppid 1), which the next run's backstop kills before booting. A normal run passes (5/5) with no qemu left.

## Phase 9a: Play Store root — done as a beta pass (2026-10-14)

Abdurahman accepted ADR-0002 option (3) ("yes to both, accept option 3"): the guided tap ships as the Play Store beta for the MVP, built in 9d. Options (1) and (2) are deferred to post-MVP, to be revisited by hand. ADR-0002 is Accepted; 9a is Done.

---

## Phase 11a rework for D41 (Workbench design) — plan (2026-10-14)

Phase 9d is not being built in this session: a safety check stopped this session twice when it started on it, so 9d (and 9e and 11b, which need it) goes back to Abdurahman. This rework doesn't depend on it.

1. `site/scripts/build-styles.mjs`, run as `predev`/`prebuild`:
   - `src/styles/tokens.css` generated from `docs/design/tokens.json`: colours for dark (`:root`, `.jidr-dark`) and light (`prefers-color-scheme`, `[data-theme]`, `.jidr-light`), font families and weights, every type role expanded (family/size/weight/line-height/letter-spacing, `-ar` from the Arabic group, `features: "tnum"` for metrics), shadow parts, spacing, radius, border, size, frame, density, layout, duration, easing. Reduced-motion overrides come from each motion token's own "Reduced:" note. No hand-copied values.
   - `src/styles/workbench.css`: the component rules of `docs/design/screens/jidr.css` (everything after its token block), pruned to the classes the site's `.astro` files use. Those rules only reference token variables.
   - Both files are generated, so they're git-ignored.
2. Fonts self-hosted through `@fontsource/{geist,geist-mono,noto-kufi-arabic}` (OFL-1.1): only the weights and subsets the page uses. No Google Fonts.
3. `index.astro` rebuilt to `Landing.dc.html` (phase "soon", locale "en") and checked against `Landing-Soon-390`: nav, hero, the app shot, the root showcase, features, the "Genymotion's paid features, free" table, the sponsors band, footer. The header keeps the brand symbol (D39). The Vue root-mode island goes (the design has no interactive part); the Vue integration goes with it until 11b needs it.
4. Left out, not invented: the GitHub star count, sponsor logo slots (none yet) and the العربية link (the Arabic page is 11b).
5. Validation: `npm ci && npm run build && npx @lhci/cli autorun --collect.staticDistDir=dist --assert.preset=lighthouse:recommended`, Lighthouse ≥ 95, plus a visual check at 1440 and 390 against the design.

## Phase 11a rework for D41 — done (2026-10-14)

Built:
- `site/scripts/build-styles.mjs` (+ 6 `node --test` tests). It generates `tokens.css` from `tokens.json`: all 166 themed values match the design's own `jidr.css` exactly in dark and light, after a bug the comparison caught (DTCG `{jidr-…}` aliases now become `var(--jidr-…)`, with a regression test). It also generates `workbench.css`, the design's component rules pruned to the classes the site uses (18 KB). Both are git-ignored and generated before `dev`/`build`.
- The page rebuilt from `Landing.dc.html` (concept 8A, "soon"): nav with the brand symbol, hero, the app shot (inert, labelled as an image), root showcase with the Play Integrity banner, features, the Genymotion comparison table ("Plans checked 14 October 2026"), sponsors band, footer. Markup copied from the design file by script, not retyped.
- Fonts self-hosted from `@fontsource` (Geist 400–700, Geist Mono 400/500, Noto Kufi Arabic 600 Arabic subset) and preloaded. Google Fonts and the Vue island/integration are gone.

Validation: `npm ci && npm run build && npx @lhci/cli autorun --collect.staticDistDir=dist --assert.preset=lighthouse:recommended` → all assertions pass, 100/100/100/100 on three runs, FCP 0.7 s. The first run failed two error-level audits, both fixed: heading order (the design's `h4`/`h5` now carry `aria-level="3"`) and the font request chain (fonts preloaded; FCP went from 1.8 s to 0.7 s). Checked in the browser at 1440 (1180 px column, hero 64/66 Geist 700, no overflow) and 390 (hero 40 px, side panels and nav links hidden as designed, no overflow).

Left out on purpose: the star count, the sponsor logo slots (no sponsors yet) and the العربية link (the Arabic page is 11b). Gap G13: `tokens.json` lacks the Arabic web type sizes that 11b needs.

## Emulator test cleanup: switched to Stream A's helpers (2026-10-14)

Merged `stream/a-core` (`0f32814`: Stream A's leak fix `9ee8eed`, main and the same rustfmt fix, merged cleanly). `crates/jidr-root/tests/adb_emulator.rs` now uses `jidr_backend::testing`: a per-process device name (`unique_avd`), an orphan sweep at start (`kill_orphans`) and `AvdGuard`, whose reaper process also covers a SIGKILLed test. Stream C's own guard and backstop are gone. Checked: a deliberately failing run and a passing run (5/5; cold boot 24.4 s, live adb root 1.4 s, cold boot + hook 15.4 s) both leave no qemu. Asked Stream A to add `binary(adb_emulator)` to the 1-thread `emulator` test group in `.config/nextest.toml`.

## Phase 9d: Magisk mode — done (2026-10-14)

Built (`crates/jidr-root/src/magisk/**`, `assets/magisk-pins.toml`, `docs/root.md`, `scripts/e2e-root.sh`; deps added to `jidr-root`: `jidr-sdk`, `jidr-cpio`, `toml`, `zip`):

- **Pin + fetch** (`pins.rs`, `assets/magisk-pins.toml`): one entry — Magisk v30.7, its official GitHub release URL, SHA-256 `e0d32d21…ebd5`, licence `GPL-3.0-only` and source link. `resolve_apk` gets the APK through `DownloadCache::fetch` (SHA-256 verified) and shows the GPL notice once, only before the first download. `root.set` takes `magisk_sha256` (the CLI imports `--magisk <path>` via `cache.import` first), which uses the cached copy offline (D30).
- **Per-device patch** (`patch.rs`): hashes the pristine `ramdisk.img`, extracts the payload from the APK (`lib/<abi>/libmagiskinit.so` → `init`, `libmagisk.so`, `libinit-ld.so`, `assets/stub.apk`), xz-compresses it (preset 6, CRC32 — what `magiskboot compress=xz` uses), edits the first cpio segment (`init` → magiskinit, `overlay.d/sbin/{magisk,stub,init-ld}.xz`, `.backup/.magisk`), and writes `<avd>/jidr/ramdisk.magisk-<ver>.img` atomically (temp + rename). Provenance (version + pristine SHA-256) lives in a `.json` sidecar; the SDK image is never written.
- **State machine** (`mod.rs`): `to_magisk` = fetch → patch → set `launch.ramdisk` + `force_cold_boot` → cold boot → install app → `/data/adb/magisk` env setup (`env.rs`) → one reboot → shell grant (`grant.rs`) → verify. `clear_patch` reverts (delete file + clear launch → cold boot). Every step reports progress and checks cancel; a failure sets `health: failed` and leaves the device revertable. `on_booted` re-patches (from cache only) when the pristine hash or pin changed — patches never stack.
- **Shell grant** (`grant.rs`, D22): zero-touch on `google_apis`/`default`/`aosp_atd` (adb root → `magisk --sqlite "REPLACE INTO policies … VALUES (2000,2,…)"` → drop root); Play Store uses the ADR-0002 guided tap (`su -c id` poll, 60 s timeout, `awaiting_user_grant` on deny/timeout). `auto_grant_all` stays opt-in/off.
- **Verify** (`verify.rs`): `su -c id` → uid 0 plus `su -c 'magisk -v'`; the version rides on `Verification.magisk_version`.
- **`root.module.install {path}`** (D35/R12): Magisk-only (usage error otherwise), destructive + locks device; pushes the zip, `magisk --install-module`, cold boot, verify.
- **Notices**: `root.status`/`root.set` both carry `play_integrity` ("Play Integrity cannot pass…").

Tests (all offline, `cargo xtask ci` → 202 passed):
- `magisk::patch` unit: patch idempotence/determinism, refuses an already-patched ramdisk, atomic write + provenance round-trip, ABI mapping.
- `magisk::env` unit: environment extraction (lib→binary name, assets, chromeos) and wrong-ABI rejection.
- `magisk::grant` unit: the guided grant's 60 s timeout → `awaiting_user_grant`, and success once su answers root.
- `tests/magisk_mode.rs`: full `root.set magisk` flow roots and verifies (`su -c id` uid 0, `magisk -v`), SDK ramdisk hash unchanged, re-patch on pristine change, revert deletes the file + clears `launch.ramdisk`, `root.module.install` refuses non-Magisk, command flags (destructive/locked/long-running) and the `magisk_sha256` param. The fake adb in `tests/common` now simulates `su`, the `magisk` binary and the su-policy DB, and the fake SDK ships a real cpio ramdisk + a synthetic Magisk APK.
- `tests/magisk_emulator.rs` (behind `emulator`): none → magisk → none on a real emulator, verifying each step and the SDK hash; compiles and mirrors `adb_emulator.rs` (AvdGuard/unique_avd/kill_orphans).

Clean-room: the ramdisk layout, payload/ABI layout, `/data/adb/magisk` environment and the su-policy grant were read from Magisk's `boot_patch.sh` (GPL, read-only) and ported from avdroot (MIT, attributed in `NOTICE` and `docs/clean-room.md`). No GPL code copied.

Open issues:
- The real-emulator validation (`scripts/e2e-root.sh` on API 30/33/35 x86_64, `cargo nextest run -p jidr-root --features emulator`) needs Linux KVM CI (or `xtask e2e-mac` on arm64), which can't run until the branch is pushed. The patch is faithful to the documented layout but has not been boot-tested here (no emulator + network in this session).
- The Play Store beta path is specified (ADR-0002) but its env setup relies on the Magisk app; the guided tap is exercised offline at the grant level, not end-to-end on a playstore image.
- `SystemImage.pristine_ramdisk_sha256` is filled by the root engine only implicitly (provenance sidecar); surfacing it in `image.list`/`device.details` is a small Stream A follow-up (gap G14).

## Fix: Magisk device never booted (arm64 e2e) — 2026-10-14

The arm64 e2e (`scripts/e2e-root.sh`) hung at `set-magisk`: the first boot with
`launch.ramdisk` never reached adb (300 s boot timeout). Root cause: magiskinit
re-execs the *original* first-stage init from `.backup/init` at boot; the patch
replaced `init` with magiskinit but never recorded the stock copy, so Magisk's
restore step fell back to symlinking `/init -> /system/bin/init` and skipped the
emulator's real (2.9 MB) first-stage init.

Fix (`crates/jidr-root/src/magisk/patch.rs`): ported `magiskboot cpio backup`
— store changed/removed stock entries under `.backup/<name>.xz` and list added
entries in `.backup/.rmlist`. Regression test added. Also registered
`jidr_root::provider()` in the CLI daemon (G7/G8) and made `e2e-root.sh` keep
`JIDR_HOME` on failure (`JIDR_E2E_KEEP=1`) and use the correct `--root` flag.

Validated: `cargo xtask ci` (203 tests) and a real arm64 API 31 `google_apis`
run — none → adb → magisk → none in 255 s, `su -c id` = uid 0, `magisk -v`
`30.7:MAGISK:R`, SDK ramdisk hash unchanged.

## CI fix for PR #1's emulator jobs (2026-10-14)

Run 38561510057 failed all three Linux KVM emulator jobs. Fast-forwarded `stream/c-root` to `5d8fec1` (prepared as draft PR #5, reviewed here):
- `crates/jidr-root/tests/adb_emulator.rs`: dropped the step asserting `root.set {mode: magisk}` is refused. That was true before Phase 9d; Magisk mode now works and is covered by `magisk_emulator.rs`.
- `crates/jidr-mcp/tests/agent_tasks.rs` (Stream A's file, test-only): taps Wait on a launcher "isn't responding" dialog during the agent tasks (API 30 CI), with a unit test.
