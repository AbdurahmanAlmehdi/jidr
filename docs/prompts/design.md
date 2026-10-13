# Design prompts

Attach `docs/design/PRODUCT.md` to every prompt. Run them in order: **1 → (you pick a direction) → 2 → 3**, then **4** (icon) once prompt 2 has fixed the colours. Fill in the `[…]` placeholders with links from the earlier steps. Save outputs (tokens.json, exported screens, the icon set) into `docs/design/` so the build agents pick them up.

---

## 1. Visual directions (app + landing page)

```
I'm designing Jidr (جذر, pronounced JID-er, Arabic for "root"), a free, open-source Android emulator for the Mac. It's a desktop app (plus a CLI and an AI-agent/MCP server) that creates and runs virtual Android phones and tablets. It does what Genymotion Desktop does (device manager, a device window, sensor controls such as GPS, battery and network), but everything is free, and its headline feature is one-click rooted devices: pick "Full root" when you create a device and, about a minute later, it shows "Rooted ✓ · Magisk 30.7". I've attached PRODUCT.md, which describes exactly what it does, who uses it, the data the app really has, the statuses, and sample data. Read it fully first: it's the source of truth.

First I need visual directions, then a design system, then concepts for the key screens and the landing page. Work in the steps below and stop where I ask you to.

Research first
This is a macOS desktop developer tool, not a mobile app. Study well-regarded desktop dev tools and Mac apps, and cite the specific screens that informed each decision: Xcode's Simulator and Devices window, Android Studio's Device Manager and Running Devices, OrbStack, Docker Desktop, Proxyman, Tower, Linear (desktop), Raycast, Zed, Arc, CleanShot X, and Genymotion's public screenshots and docs (for structure only: never copy its look, and use no Genymotion branding). If you have the AppLlama MCP, it covers iOS apps; use it only for mobile-device framing ideas and say so in one line. For the landing page, study open-source dev-tool sites such as OrbStack, Zed, Ghostty, Warp, Bun and Tauri.

References I'm attaching
[optional: screenshots or links you like]. Take the direction from these, not the content. Tell me what you borrowed from each.

The key surfaces (for later steps; read now so the directions fit them)
The content listed for each is a starting brief, not a spec. Use UX best practice and your research to decide what belongs on each screen: add what's missing, cut or move what doesn't earn its place, and tell me what you changed and why.

Non-negotiables (keep these whatever else you change):
- The Android screen is the hero of the device window. Chrome around it is quiet and narrow, and nothing ever overlaps the device screen.
- Root state is unmistakable wherever a device appears: label plus icon, never colour alone. Verified, unverified and failed must look different.
- Device status (Stopped, Booting Android… 62%, Running, Crashed, Busy · in use by CLI) is always visible on device rows.
- Keyboard-first: every action reachable from shortcuts or a ⌘K command palette.
- Native macOS feel: traffic lights, standard menus, sheets and popovers. Dark mode is first-class (many users live in dark IDEs).
- Buildable in Slint (a Rust GUI toolkit): solid fills, borders, rounded rects, simple shadows, images and vector paths. No heavy blur or vibrancy dependence, no web views.
- No Google, Android or Genymotion logos or trademarks anywhere. Use our own neutral device silhouettes.
- English UI at launch, but every layout must mirror cleanly for Arabic RTL later.

Surfaces:
- Launcher: the device library (status, Android version, root badge, disk use, running jobs such as an image download), create, start, stop, clone, delete.
- First-run setup: no SDK found → Google's license (must be scrolled and explicitly accepted) → download progress → first device.
- Create device: profile, Android version, root mode (None / Shell root / Full root, with an honest one-line explanation of each), RAM, CPU, screen.
- Device window: the live screen, a narrow toolbar (volume, rotate, back, home, recents, power, screenshot, record, pixel-perfect) and a widget panel (battery, GPS with a map, network, phone/SMS, motion sensors, dev tools, snapshots, logcat).
- Root setup flow: download Magisk → patch → reboot → install → verify, then success or failure with one-click revert.
- Landing page (jidr.ly): coming-soon now, the full site at launch.

Sample data
Use the sample data in PRODUCT.md §7 (the devices "Pixel-ish 6.1", "Tablet 11", "Play Store test" and "CI-runner-03"; the API 36 download job; Tripoli GPS coordinates; snapshots; logcat lines) so the screens feel real and the numbers agree.

Step 1: visual directions (then STOP)
Propose 3 clearly different visual directions. They should differ in structure and personality, not just colour. For each one:
- a name and a one-line idea;
- palette (light and dark), typography (a Latin UI face, a monospace for serials, ports and logs, and an Arabic face for the future RTL version) and shape language;
- the Launcher drawn in that direction at 1440×900, in two states: first run (no devices, SDK not installed) and populated (the four sample devices, "Pixel-ish 6.1" running with "Rooted ✓ · Magisk 30.7", the API 36 download in progress);
- the device window for "Pixel-ish 6.1" with the GPS widget open;
- a landing-page hero (desktop 1440 and mobile 390) in the same direction.
Base each direction on patterns from real, well-regarded apps and say which app inspired what. Then stop and let me choose.

Constraints for every direction:
- Feels like a precise professional tool you'd trust with your device and your root access, but warm and welcoming for someone who has never installed the Android SDK. Not a hacker-green terminal cliché, not a generic SaaS dashboard.
- The "root" idea (جذر) can inform the visual language subtly. Don't put trees everywhere.
- Text contrast of at least 4.5:1, visible focus rings, nothing conveyed by colour alone.
- Dense enough for power users at 1440×900, readable at 1280×800.
- No emoji in the interface, no gradient washes, no stock imagery, no fake 3D phones.
- There is no logo yet (it comes after the design system), so don't depend on one; a plain wordmark is fine.

Be opinionated. Tell me which direction you'd pick and why.
```

---

## 2. Design system

```
Going with direction [NAME] for Jidr (جذر) from the [Jidr — Visual directions] design: [link]. Build the design system as a reference page, with light and dark versions side by side. PRODUCT.md is attached again; it's the source of truth for statuses and data.

It serves two products from one source: the Slint desktop app (Rust) and the Astro landing page (CSS). Name every token so it maps one to one onto a Slint global property and a CSS custom property (e.g. color.surface.raised → Theme.surface-raised in `export global Theme { … }` → --jidr-color-surface-raised). Ship the tokens as a tokens.json too (W3C design-tokens format), plus a generated theme.slint and tokens.css.

Tokens
- Colour (semantic names, never raw hues in components): surface (base, raised, sunken, overlay, device-bezel), text (primary, secondary, tertiary, on-accent, disabled, code), border (subtle, default, strong), accent (and on-accent), positive, negative, warning, info, focus ring, scrim, selection.
- Status tokens: one colour pair (container and on-container) plus an icon and a label for every status in PRODUCT.md §5. Device: stopped, starting, booting, running, paused, stopping, crashed, busy. Root: not rooted, shell root, full root, setting up, verified, unverified, failed, beta. Job: queued, running, done, failed, cancelled. Statuses that mean the same thing to the user can share a pair; say which. Root states must be distinguishable without colour.
- Logcat level tokens (V, D, I, W, E, F) that stay readable on both themes.
- Type scale: a Latin UI face (primary), a monospace, and an Arabic face for the future RTL version. Roles: display, headline, title, body, label, caption, plus:
  - code: serials, ports, package names, CLI snippets (emulator-5556, com.example.shop);
  - metric: numbers that update live (battery 64%, 62% boot, 1.4 / 2.1 GB, coordinates 32.8954, 13.1802), using tabular figures;
  - hero: the one big thing a surface is for (on the landing page, the headline; in the app, a device name in its detail view or the root result).
  Say how Arabic and Latin line heights differ for the RTL version.
- Spacing, radii, elevation (light and dark: in dark, elevation is surface tint, not shadow), borders, the device-screen frame (bezel thickness and radius per device class, no brand silhouettes).
- Motion: durations, easings, and when motion is used (sheet and popover, panel slide, status change, job progress, root success, skeletons). Every motion has a Reduce Motion fallback. Keep it subtle: this is a tool.
- Density: a default and a compact density for the device list.
- Breakpoints for the landing page (390 / 768 / 1280 / 1440) and minimum window sizes for the app (launcher 960×600; device window sized to the device).

Components (desktop app)
Show each with all its states (default, hover, pressed, focused, disabled, loading, error) in light and dark, LTR plus one RTL sample:
- window chrome: title bar with traffic lights and toolbar, sidebar, split view, inspector panel;
- device row (list) and device card (grid), with status, root badge, Android version, disk use, inline actions;
- root badge (every root state), status pill (every device state), job progress row (download / patch / verify, with cancel);
- root-mode selector: three cards (None / Shell root / Full root) with one-line explanations and a "Beta" marker for Play Store images;
- device window: screen frame, vertical toolbar of icon buttons with tooltips and shortcut hints, widget panel host with a tab or segmented switcher;
- widget building blocks: slider with value field (battery), toggle, segmented control, numeric field with units (lat/long/alt), map container with OSM attribution, file drop zone (GPX/KML, APK), key pad (D-pad, media keys);
- logcat viewer (level filter, search, monospace rows, follow toggle);
- snapshot list with thumbnail, root-mode tag, size, actions;
- license consent sheet (scrollable text, "I accept" enabled only after scrolling, decline);
- create-device sheet (stepper or single form; choose and argue it);
- command palette (⌘K), menu, context menu, popover, tooltip, toast, dialog, confirm-destructive dialog (typed name for delete), empty state, error state, inline error, skeletons;
- buttons (primary, secondary, tertiary, destructive, icon), text input, search field, dropdown/select, checkbox, radio, keyboard-shortcut hint (kbd).

Components (landing page)
Nav with GitHub star count, hero, feature grid, comparison table ("Genymotion's paid features, free in Jidr": text only, no logos), root showcase block, agent/MCP block with a code sample, CLI code block with copy, download button (macOS arm64) with checksum, sponsors strip, footer with licences, an Arabic RTL sample of the hero.

Rules to write down
- How many accent things a surface may have (one hero per surface).
- Icon set: Lucide (ISC licence; record attribution), outlined vs filled rules, sizes (14/16/20), and RTL mirroring rules (which icons flip: arrows, back/forward, progress; which don't: checks, play, record, device rotation).
- Number formatting (tabular figures, units, GB vs GiB: argue it), coordinates, timestamps in logcat vs UI.
- How the device screen is framed at any window size (fit, pixel-perfect 1:1, letterboxing colour).
- Copy rules for honest states (e.g. "Rooted emulators can't pass Play Integrity").
- Contrast of every text/background token pair (list the ratios), including status and logcat tokens on both themes.
```

---

## 3. Key screens: app + landing page

```
Using the Jidr (جذر) design system ([link]) and the [NAME] direction I chose ([link]), design the key screens of each part of the product so I can see how the whole thing feels in practice and finalize the structure. PRODUCT.md (the product, the data the daemon really returns, the sample data) and the logo and icon (if ready) are attached.

Ground rules
- Desktop app: macOS, Slint, 1440×900 (and check 1280×800). Light and dark for everything in the app.
- Landing page: 1440 and 390 wide, light (dark if the direction calls for it), plus the hero in Arabic RTL.
- Design within the data in PRODUCT.md §6. If a screen would really benefit from something listed as "not available", draw it as a clearly labelled optional variant, never in the main version.
- Use the status tokens, icons and labels from the design system. If you need a new token or component, list it at the end instead of inventing it inline.
- One hero per surface: in the device window it's the Android screen; in the root flow it's the root result; on the landing page it's the headline.
- No emoji, no gradient washes, no stock imagery, no Google, Android or Genymotion logos.

The screens
1. App: Launcher. Draw 2 structurally different concepts:
   A. Sidebar device list + device detail inspector.
   B. A grid of device cards with a command bar.
   Both in two states: first run (empty, SDK not installed) and populated (the four sample devices, one booting at 62%, the API 36 download running, "CI-runner-03" busy in use by the CLI). Then pick one and argue it.
2. App: First-run setup. SDK detection result, Google's license consent (scroll to enable accept), download progress with time left, and "Create your first device".
3. App: Create device. Profile, Android version (only images that run on Apple Silicon), root mode cards (None / Shell root / Full root, with "Beta" on Play Store images and the Play Integrity note), RAM, CPU, screen, name. Show the default (Shell root preselected, Full root highlighted).
4. App: Device window. Draw 2 concepts:
   A. A docked widget panel beside the screen.
   B. A floating inspector that detaches.
   Then, in your chosen concept, draw "Pixel-ish 6.1" running with:
   - the GPS widget (Tripoli coordinates, route replay of tripoli-airport-run.gpx at 2×);
   - the battery widget (64%, discharging);
   - the logcat panel (the sample lines, one error);
   - the drag-and-drop APK overlay;
   - pixel-perfect mode.
5. App: Root flow. Full root setup in progress (each step with its state), success ("Rooted ✓ · Magisk 30.7 · verified 2 min ago"), failure (what went wrong, logs, "Revert to unrooted" and "Try again"), and the Play Store beta variant.
6. App: Snapshots. The snapshot manager for "Pixel-ish 6.1" with the three sample snapshots, root-mode tags, the "taken with a different root mode" refusal, and the save dialog.
7. App: Settings. SDK location and disk usage per device, cache, "Add Jidr to Claude" (MCP setup with copyable config), map tile source, default root mode, about and licences (including GPLv3 for the GUI).
8. Landing page. Draw 2 concepts of the home page, then the chosen one in full:
   - coming-soon version (pitch, root showcase, "Genymotion's paid features, free", Star on GitHub, Sponsors; no email capture);
   - launch version (adds download, the agent/MCP section with a short demo transcript, the CLI snippet `jidr device create "Pixel-ish 6.1" --api 35 --root magisk --wait-boot`, docs links);
   - the Arabic RTL hero.

What I want back
- For each part, what you'd choose and why. Where I asked for 2 concepts, make them different layouts, not recolours.
- Cite the real apps that inspired each concept (e.g. Xcode Simulator, Android Studio Running Devices, OrbStack, Docker Desktop, Proxyman, Linear, Raycast; and OrbStack, Zed, Ghostty, Warp, Bun for the site).
- At the end:
  - the new tokens or components you had to add to the design system;
  - the "not available" data items worth adding to the daemon, ranked by user value;
  - anything about the product that the designs made you question.
```

---

## 4. App icon (image generation) ✅ done: outputs in `assets/brand/` (D39)

```
Design the app icon for Jidr (جذر, Arabic for "root"), a free, open-source Mac app that creates and runs virtual Android phones on your computer. Its headline feature is one-click "rooted" devices: full administrator access to the virtual phone. It's a precise, trustworthy developer tool, and the name and logo carry an Arabic identity.

The symbol
A root that grows into, or forms, the outline of a phone: the idea of a device that grows from its root. The root lines are simple, confident strokes (3–5 at most), not a realistic tree, and they resolve into a clean rounded-rectangle phone silhouette, or the phone's outline is drawn by the roots themselves. Optionally the shape of the Arabic word جذر can be echoed in the root strokes, but only if it stays legible as a symbol to non-Arabic readers; never as decoration. Not a tree, leaf, robot, Android mascot, bug, lock, padlock, terminal prompt or circuit-board pattern. No Google, Android or Genymotion resemblance.

Colours
Background: [design-system accent / surface colours, e.g. #XXXXXX → #XXXXXX as a subtle two-stop field, or a flat colour]. Symbol: [on-accent colour]. Exactly as in the Jidr design system: [link].

Format
- macOS app icon (Big Sur and later style): 1024×1024 canvas with the rounded-square ("squircle") body at about 824×824 centred, following Apple's macOS icon grid, with a soft, natural drop shadow inside the canvas. On macOS the artwork includes the rounded-square shape (unlike iOS, where the system masks it).
- The symbol is centred and takes up about 60% of the body, on the background colour or gradient. No text, no border.
- Subtle depth is fine (a gentle top-light, slight inner edge), but no glossy 3D, no photorealism, no busy texture.

It must also work
- Small: recognisable at 16×16 and 32×32 px (Finder list view, Dock at small sizes, favicon). Simplify strokes at small sizes if needed and show the 16 px version.
- In one colour: give a version as a single-colour symbol on transparent, for the macOS menu bar (template image, 18 pt), the website favicon, GitHub's org avatar and in-app use.
- On a circle: the symbol alone inside a circle for the GitHub org avatar and social previews (nothing important near the edges).
- On light and dark backgrounds: show the icon in the Dock on both a light and a dark wallpaper.

Deliver: the 1024 master, the 16/32/128/256/512 sizes, the monochrome symbol as SVG-ready flat shapes, the circular avatar, and a 1200×630 social preview card with the icon and the wordmark "Jidr" + "جذر".
```
