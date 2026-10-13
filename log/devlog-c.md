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
