# /goal prompts: three work streams

Run each in its own Claude Code session, in its own git worktree of `~/dev/personal/lbytech/jidr`. Start **A** now. Start **C** now too: its day-1 phases (9a, 9b, 11a) don't need A. Start **B** once A has tagged `protocol-v0.1` and written ADR-0001.

Before starting B or C, make sure A has committed Phase 0 (the workspace skeleton), so their worktrees branch from it.

---

## Stream A: Core platform

```
/goal Build Stream A ("Core platform") of Jidr (جذر), an open-source Genymotion Desktop alternative in pure Rust with one-click rooted devices, in this repo (~/dev/personal/lbytech/jidr, worktree branch `stream/a-core`).

Read first, fully: docs/plan/2026-10-13-feat-jidr-emulator-suite-plan.md (the source of truth, especially "Decisions log" D1–D34, the crate table, the protocol section and "Implementation Phases") and docs/research/2026-10-13-research-summary.md. Read the AOSP emulator protos at ~/Library/Android/sdk/emulator/lib/emulator_controller.proto and snapshot_service.proto.

Your phases, in order: 0, 1, 2, 3a, 3b, 3c, 4a, 4b, 4c, 5a, 5b, 6. You own: root Cargo.toml, xtask (except xtask/src/release.rs), jidr-core, jidr-proto, jidr-adb, jidr-sdk, jidr-backend, jidr-daemon, jidr-client, jidr-mcp, apps/jidr, .github/workflows/ci.yml. Never edit another stream's files. Phase 0 must stub every crate in the plan's crate table (including B's and C's) so the other streams never touch the root Cargo.toml.

How to work:
- Before each phase, write a short plan for it in log/devlog-a.md, then implement it.
- Done means the phase's **Validation** command exits 0 and its acceptance criteria hold. Then set its **Status:** to Done in the plan, commit with a conventional-commit message (one commit per phase, local only), and append a devlog entry (what was built, what was skipped, open issues).
- Use CARGO_TARGET_DIR=target/agent-a. Run `cargo xtask ci` before every commit.
- Never `unwrap`/`expect`/`panic` outside tests. Every bug fix gets a regression test.
- Emulator tests: use `--features emulator`. Locally use arm64 images (`google_apis`, API 30/33/35). CI uses Linux KVM with x86_64 images (D28).
- Phase 1 is a go/no-go. If the Slint display spike misses ≥30 fps / ≤80 ms after trying inline → MMAP → wgpu import, record it in ADR-0001 and continue. Stream B reads that decision.
- After 4c, create the git tag `protocol-v0.1`. Protocol changes after that need an entry in docs/control-protocol.md's changelog.
- Check docs/gaps.md at the start of every phase: streams B and C file requests for daemon/CLI/backend hooks there. Serve them before starting a new phase if they block someone.

Clean-room rules (non-negotiable): use only public Genymotion docs and the public AOSP emulator protos/docs. Never decompile, string-dump or inspect Genymotion binaries. Never copy GPL code (Magisk, rootAVD). Never bundle Google SDK binaries or system images; download them on the user's machine only after explicit license consent. No Google, Android or Genymotion logos.

Stop and ask me (don't guess) before: pushing to any remote, creating GitHub repos or orgs, spending money, accepting a Google SDK license on my behalf in a non-test context, or changing a decision in the Decisions log. If a feature depends on a 🔎 NEEDS RESEARCH row, skip it and list it in docs/gaps.md.

The goal is complete when Phases 0–6 all show Status: Done, `cargo xtask ci && cargo nextest run -p jidr-daemon -p jidr-client && scripts/e2e-cli.sh && cargo nextest run -p jidr-mcp --features emulator` passes, the tag protocol-v0.1 exists, and log/devlog-a.md ends with a summary of what's done, what's deferred and the open gaps.
```

---

## Stream B: Player & widgets

```
/goal Build Stream B ("Player & widgets") of Jidr (جذر), an open-source Genymotion Desktop alternative in pure Rust (Slint GUI) with one-click rooted devices, in this repo (~/dev/personal/lbytech/jidr, worktree branch `stream/b-player`, branched from the commit tagged protocol-v0.1).

Read first, fully: docs/plan/2026-10-13-feat-jidr-emulator-suite-plan.md (source of truth: "Decisions log" D1–D34, crate table, protocol and display-pipeline sections, Implementation Phases), docs/decisions/0001-frame-transport.md (Stream A's display spike: use the transport and rendering path it chose), docs/control-protocol.md, docs/design/PRODUCT.md, and docs/design/ (the design system, tokens.json and screen designs, if present).

Precondition: the git tag protocol-v0.1 and ADR-0001 must exist. If not, stop and tell me.

Your phases, in order: 7a, 7b, 7c, 8a, 8b, 10a, 10b. You own: crates/jidr-ui (except src/root/**, which is Stream C's), apps/jidr-gui, crates/jidr-widgets. Never edit the root Cargo.toml, jidr-daemon, apps/jidr or other streams' crates. If you need a daemon command, hook or protocol change, add a row to docs/gaps.md (Needs | From stream | Blocking phase) and work on something else until Stream A delivers it.

How to work:
- Before each phase, write a short plan in log/devlog-b.md, then implement it.
- Done means the phase's **Validation** command exits 0 (or the manual steps are written up in the devlog with numbers) and its acceptance criteria hold. Then set **Status:** Done in the plan, commit locally (one conventional commit per phase) and append a devlog entry.
- Use CARGO_TARGET_DIR=target/agent-b.
- The GUI is GPLv3 (D18); jidr-widgets is Apache-2.0 and must not depend on Slint.
- The UI has no business logic: every action is a protocol call through jidr-client. Pure view state (window size, zoom, panel layout) stays local.
- Styling: use docs/design/tokens.json and the design system when they exist. Until then use Slint's default style with theme tokens in one `global Theme` so restyling is a single-file change. Light and dark, every string via @tr, layouts checked with RTL mirroring (D33).
- Golden snapshot tests for every screen and state, in light, dark and RTL. Verify visual changes with the offscreen snapshot harness before committing.
- Widgets: only the MVP set in Phase 8a/8b. Identifiers, audio injection, biometrics and every 🔎 NEEDS RESEARCH row are out of scope.
- Use the sample data in docs/design/PRODUCT.md §7 for fixtures and goldens.

Clean-room rules (non-negotiable): public Genymotion docs only, never inspect Genymotion binaries, never copy GPL code, never bundle Google SDK components, and no Google, Android or Genymotion logos or trademarks in the UI.

Stop and ask me before pushing to any remote, changing a Decisions-log entry, or adding a dependency with a non-permissive licence to jidr-widgets.

The goal is complete when Phases 7a–8b and 10a–10b all show Status: Done, `cargo nextest run -p jidr-ui -p jidr-widgets && cargo run -p jidr-ui --example snapshot -- --check && cargo nextest run -p jidr-widgets --features emulator` passes, the Phase 7c manual player check is recorded in log/devlog-b.md with fps and latency numbers, and the devlog ends with a summary of what's done, what's deferred and the open gaps.
```

---

## Stream C: Root, ship & site

```
/goal Build Stream C ("Root, ship & site") of Jidr (جذر, "root"), an open-source Genymotion Desktop alternative in pure Rust whose headline feature is one-click rooted Android devices, in this repo (~/dev/personal/lbytech/jidr, worktree branch `stream/c-root`).

Read first, fully: docs/plan/2026-10-13-feat-jidr-emulator-suite-plan.md (source of truth: "Decisions log" D1–D34, the "Root engine" section, crate table, Implementation Phases), docs/research/2026-10-13-research-summary.md (the root research), docs/design/PRODUCT.md and docs/design/ (brand kit, tokens, landing-page designs, if present).

Your phases, in this order:
1. Day 1 (no dependency on Stream A): 9a (Play Store root R&D spike, time-boxed to about 3 agent-days, no product code), 9b (jidr-cpio), 11a (coming-soon site).
2. After the tag protocol-v0.1 exists: 9c, 9d, 9e, 12a, then 11b once the 9d Magisk demo works.
3. 12b stays Blocked until I add the Apple Developer ID secrets. Prepare the scripts, but don't mark it Done.

You own: spikes/playstore-root, crates/jidr-cpio, crates/jidr-root, crates/jidr-ui/src/root/** only, assets/magisk-pins.toml, docs/root.md, scripts/e2e-root.sh, site/, packaging/, xtask/src/release.rs, .github/workflows/release.yml. Never edit the root Cargo.toml, jidr-daemon, jidr-backend, apps/jidr or other streams' files. Request the hooks you need (e.g. LaunchOptions.ramdisk, the Magisk download cache, snapshot root tags) via docs/gaps.md (Needs | From stream | Blocking phase).

Root rules (D6, D7, D21, D22 and the research summary):
- Never modify the shared SDK ramdisk.img. Write a per-device patched copy and launch with -ramdisk.
- Never bundle Magisk (GPL-3.0). Download pinned official GitHub releases at runtime, verify SHA-256, cache by hash, support --magisk <path>.
- Clean-room: you may READ Magisk's and rootAVD's scripts to learn the ramdisk layout, but never copy or translate GPL code. avdroot (MIT) may be ported with attribution in NOTICE.
- The adb shell is auto-granted su; apps get Magisk's normal prompt; the opt-in "auto-grant all" toggle is off by default.
- Root-mode changes force a cold boot; snapshots carry the root-mode tag.
- State plainly in the UI and docs that rooted emulators can't pass Play Integrity.
- 9a: try (1) the overlay.d boot-script policy grant, (2) the uiautomator auto-tap, (3) the guided-tap beta, in that order. Write ADR-0002 with the result. Either a full-support pass or a documented beta pass completes 9a.

Site rules (D16, D25, D33, D34): Astro + Vue islands only where interactive, Cloudflare Pages. The coming-soon page has no email capture. Feature grid "Genymotion's paid features, free" is text only, with no Genymotion, Google or Android logos. Use the brand kit and design tokens in docs/design/ when present; otherwise build a clean, token-driven placeholder that's easy to restyle. Arabic version at launch (11b). GitHub Sponsors link. Lighthouse ≥ 95.

How to work:
- Before each phase, write a short plan in log/devlog-c.md, then implement it.
- Done means the phase's **Validation** command exits 0 (or the manual steps are written up in the devlog) and its acceptance criteria hold. Then set **Status:** Done in the plan, commit locally (one conventional commit per phase) and append a devlog entry.
- Use CARGO_TARGET_DIR=target/agent-c. No unwrap/expect/panic outside tests.
- Root tests: Linux KVM x86_64 in CI; arm64 API 30/33/35 locally on this Mac.

Stop and ask me before: pushing to any remote, deploying the site, connecting domains, creating GitHub orgs/repos/Sponsors, spending money, or changing a Decisions-log entry.

The goal is complete when 9a–9e, 11a, 11b and 12a show Status: Done, 12b is prepared and marked Blocked, `cargo nextest run -p jidr-cpio && scripts/e2e-root.sh && cargo xtask release --dry-run --scan && (cd site && npm ci && npm run build)` passes, ADR-0002 records the Play Store result, and log/devlog-c.md ends with a summary of what's done, what's blocked on me and the open gaps.
```
