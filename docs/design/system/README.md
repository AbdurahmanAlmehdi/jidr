Jidr (جذر, *JID-er*, "root") is a free, open-source Android emulator for the Mac: a desktop app built in Slint (Rust), a CLI and an MCP server, plus the jidr.ly landing page built in Astro. This system is direction **Workbench**: one window like an IDE, dark-first, soft cards, devices open as tabs. Both products read the same tokens.

## One source, two products

- `tokens.json` here mirrors the product's W3C design-token file. Every name is the product's CSS custom property without the leading `--`: `jidr-color-surface-raised` is `--jidr-color-surface-raised` on the site and `Theme.surface-raised` in Slint (a leading `color-` is dropped in Slint; everything else is the same path).
- Never put a raw hex in a component. Read colours through semantic tokens, and status colours through `jidr-status-<kind>-<state>-*`, never through the shared pairs or `jidr-color-positive` directly.
- The two themes are `dark` (primary) and `light`. Design dark first, then check light; do not invert one into the other.
- Typography composites (`--jidr-type-<role>-size`, `-line-height`, `-weight`, `-letter-spacing`, `-family`) are mirrored in `components/bundle.css` from the type styles below. Reduce Motion overrides live there too.

## Voice and copy

Confident, technical, warm. Short labels, sentence case, no exclamation marks, no emoji in the UI, no "Oops".

- Say what happens and what it costs: "Reboots the device to finish root setup (about 40 s)." Buttons are verbs that match the result; the toast confirms in the past tense ("Deleted Tablet 11").
- State limits plainly where the user decides, not in a help page: "Rooted emulators can't pass Play Integrity." goes on the Full root card and in the inspector.
- Progress is honest: real bytes or real steps, never a fake bar. "Rooted ✓" means Jidr ran `su -c id` and got uid 0; until then the badge says Full root or Root unverified.
- Errors say what went wrong, whether data is safe, and what to do: "Root failed: patch step timed out. The device is unchanged."
- Status labels come from PRODUCT.md §5 exactly: Stopped · Starting… · Booting Android… · Running · Paused · Stopping… · Crashed · Busy (in use by CLI/agent); Not rooted · Shell root (adb) · Full root · Setting up root… · Rooted · Magisk 30.7 · Root unverified · Root failed: reason · Beta.
- No Google, Android or Genymotion logos or trademarks as graphics. "Android" appears only as text naming the OS.

| Write | Not |
|---|---|
| Downloading Android 16 (API 36) image · 1.4 / 2.1 GB · 2 min left | Please wait… |
| Busy · in use by CLI | Unavailable |
| Jidr downloads Google's Android Emulator after you accept Google's license. | Jidr includes the Android Emulator. |

## Colour

- Grounds, back to front: `jidr-color-surface-base` (content), `jidr-color-surface-chrome` (title bar, tabs, sidebar, device toolbar), `jidr-color-surface-raised` (cards, inspector), `jidr-color-surface-control` (buttons, inputs; `-hover`, `-pressed`), `jidr-color-surface-overlay` (popovers, menus). Wells are `jidr-color-surface-sunken` (device stage letterbox, code, map, tracks).
- Text: `jidr-color-text-primary`, `-secondary`, `-tertiary` all hold 4.5:1 on every surface in both themes; each token's note lists the ratios. `jidr-color-text-disabled` is exempt and only used with a non-interactive control.
- Code, serials and package names: `jidr-color-text-code` on `jidr-color-surface-sunken`.
- Accent blue `jidr-color-accent-fill` is for action and selection only. Text on it is `jidr-color-text-on-accent`; hover and pressed get darker in both themes so the pair keeps 4.5:1.
- Root violet `jidr-color-root` means root and nothing else: never a button, link, selection or chart colour. The only solid root plate is the verified badge (`jidr-status-root-strong-*`).
- Control outlines (inputs, checkboxes, radios) use `jidr-color-border-strong` (3:1). Dividers `jidr-color-border-subtle`; card and panel outlines `jidr-color-border-default`.
- Focus: a solid 2 px `jidr-color-focus-ring` outline, 2 px offset (`jidr-border-width-focus`, `jidr-border-focus-offset`), on every focusable element. It holds at least 5:1 on every surface.
- Behind sheets and dialogs: `jidr-color-scrim`. Text selection and selected logcat rows: `jidr-color-selection`.

## Status

Every status has a colour pair, an icon and a label. Statuses that mean the same thing to the user share a pair:

| Pair | Means | Used by |
|---|---|---|
| `jidr-status-neutral-*` | idle or ended by the user | Stopped, Paused, Queued, Cancelled, Not rooted, Shell root, SDK Not installed |
| `jidr-status-positive-*` | working as intended | Running, Done, SDK Ready |
| `jidr-status-transition-*` | wait, something is happening | Starting, Booting, Stopping, job running, Installing, Update available |
| `jidr-status-warning-*` | usable with a caveat | Busy, Beta, Needs license acceptance |
| `jidr-status-negative-*` | broken, needs action | Crashed, Root failed, job Failed |
| `jidr-status-root-soft-*` | root configured, not yet proven | Full root, Setting up root, Root unverified |
| `jidr-status-root-strong-*` | root verified | Rooted · Magisk 30.7 |

- Root states are told apart without colour: verified is the only solid plate (badge-check); Full root, Setting up and Unverified are outlined pills with key-round, spinner and circle-help; Failed has a 2 px heavy outline and circle-x plus its reason; Shell root is a filled neutral pill with the terminal glyph; Not rooted is text only with a minus. Beta is a square uppercase tag after the badge, never alone.
- Device status is visible wherever a device appears: rows, cards, tabs, the device window title. Booting carries its percentage, Running its uptime, Busy names its holder.
- No dashed borders anywhere: Slint can't draw them.

## Logcat

Level colours `jidr-logcat-verbose`, `-debug`, `-info`, `-warn`, `-error`, `-fatal` on `jidr-logcat-background`, all at least 4.5:1 in both themes. The level letter always has its own column, and F rows get `jidr-logcat-fatal-row` plus bold, so level never depends on hue. Tag in `jidr-logcat-tag`, timestamp and pid in `jidr-logcat-meta`. Logcat stays LTR in the Arabic build.

## Type

- Geist for UI, Geist Mono for code and live numbers, Noto Kufi Arabic for the RTL build (all OFL-1.1, bundled in the app because Slint can't fetch web fonts).
- Roles: `type-hero` (the one big thing a surface is for: a device name in its detail view, the root result), `type-display` (first-run and empty-state titles), `type-headline` (page title), `type-title` (cards, sheet sections, widgets), `type-body` (13 px, macOS default), `type-label` (buttons, fields, pills, tabs), `type-caption` (metadata).
- `type-code`: serials, ports, package names, CLI (`emulator-5556`, `com.example.shop`). Always LTR.
- `type-metric` and `type-metric-lg`: anything that updates live (64%, 62% boot, 1.4 / 2.1 GB, 32.8954, 13.1802). Geist Mono, because Slint can't switch on Geist's tabular figures; mono figures are tabular by construction.
- Arabic (`type-*-ar`): line heights 1.6–1.75× the size against Latin's 1.2–1.4×, because Kufi has tall ascenders, deep descenders and stacked diacritics. Text sizes go up 1 px; display sizes come down. Letter-spacing is always 0 in Arabic: spacing breaks the joins. Row heights come from density tokens, not line height, so rows keep their height in both scripts.
- Landing page: `type-web-hero` 64/66 at ≥1280 px, 48/52 at 768, `type-web-hero-sm` 40/44 at 390; `type-web-lede` for the subhead; `type-web-body` for running text.

## Space, shape, elevation

- 4 px base: `jidr-space-xs` (4) to `jidr-space-5xl` (96). Components use `sm`/`md`/`lg`; sections `xl` and up.
- Radii: `jidr-radius-pill` for status and root, `jidr-radius-md` (8) for controls and rows, `jidr-radius-lg` (12) for cards, popovers and menus, `jidr-radius-xl` (14) for sheets and dialogs, `jidr-radius-xs` (4) for kbd and tags.
- Elevation: in dark it is surface tint plus `jidr-color-border-default` (`jidr-elevation-1|2|3-surface`, shadows transparent); in light white surfaces with one shadow each (`jidr-elevation-1|2|3-shadow`). Never stack shadows: Slint draws one per element.
- Borders: hairline 1 px everywhere; `jidr-border-width-emphasis` (1.5) only on outlined root badges.
- Density: device list default `jidr-density-default-row-height` (52, two lines) or compact `jidr-density-compact-row-height` (30, one line), toggled with ⌥⌘D.
- App window minimum `jidr-window-launcher-min-width` × `-min-height` (960 × 600). The device window is sized to the device: the screen fits 80% of the display height at first open, then resizes freely down to `jidr-window-device-min-screen-height` (360). Landing breakpoints `jidr-breakpoint-sm|md|lg|xl` (390 / 768 / 1280 / 1440).

## The device screen

- The Android screen is the hero of the device window. Nothing overlaps it; the toolbar is 48 px of icon buttons and the widget panel opens beside it.
- Bezel `jidr-color-surface-device-bezel` with `jidr-frame-<class>-bezel`, `-radius`, `-screen-radius` for phone, small-phone and tablet: a neutral rounded rectangle, no notch, camera, buttons or brand silhouette.
- Fit (default): largest size that fits, aspect kept, continuous scale. Pixel-perfect 1:1 (⌘0): one device pixel per physical pixel; the stage scrolls rather than scaling or cropping.
- Letterbox is `jidr-color-surface-sunken` with nothing drawn on it, at least `jidr-frame-stage-padding` (24) around the bezel. Rotation swaps the frame and re-fits; the window does not resize itself.

## Motion

Subtle and short; nothing bounces or runs longer than 400 ms. Every motion has a Reduce Motion fallback (in Slint `Theme.reduce-motion` from `NSWorkspace.accessibilityDisplayShouldReduceMotion`; on the site `prefers-reduced-motion`).

| Motion | Tokens | Reduce Motion |
|---|---|---|
| Hover, press, toggle | `jidr-motion-hover-*` 120 ms | unchanged (colour only) |
| Popover, menu, tooltip | `jidr-motion-popover-*` 160 ms, 4 px rise | 100 ms fade, no rise |
| Sheet | `jidr-motion-sheet-*` 240 ms, 12 px drop | 120 ms crossfade |
| Widget panel, inspector | `jidr-motion-panel-*` 240 ms | instant |
| Status change | `jidr-motion-status-*` 180 ms crossfade | instant swap |
| Job progress | `jidr-motion-progress-*` 240 ms linear | width jumps; numbers still update |
| Root success | `jidr-motion-root-success-*` 400 ms, outline fills to solid once | badge appears solid |
| Skeleton | `jidr-motion-skeleton-*` 1.4 s shimmer | flat fill |
| Spinner | `jidr-motion-spinner-*` 900 ms | static icon; label and % carry the state |

The device screen itself never animates.

## Accent and hero budget

- One hero per surface, set in `type-hero` or `type-web-hero`.
- At most one accent-filled control per surface (a pane, a sheet, a card, a landing section): the primary button or a selected segment, not both.
- Status pairs don't count against the budget, but they are never decoration: no tinted headers or cards to make a section pop.

## Iconography

- Lucide only (ISC licence; credit "Icons by Lucide, ISC licence" in About › Licenses and the site footer). Stroke 1.75 with round caps at every size. In Slint they are SVG files loaded with `@image-url` and tinted with `colorize`.
- Sizes: 14 (`jidr-icon-sm`) inside pills, badges, small buttons and menus; 16 (`jidr-icon-md`) in buttons, sidebar and inputs; 20 (`jidr-icon-lg`) in the device toolbar and empty states.
- Outlined by default. Filled is a state, not a style: only the Running dot, the record button while recording, and the toggle thumb.
- Status icons: square (Stopped), loader-circle (Starting, Booting, Stopping, Setting up, running jobs), filled dot (Running), pause, triangle-alert (Crashed), lock (Busy), minus (Not rooted), terminal (Shell root), key-round (Full root), badge-check (Rooted), circle-help (Unverified), circle-x (Root failed), flask-conical (Beta), clock (Queued), check (Done), x (Failed), ban (Cancelled).
- Mirror in RTL: arrows, back/forward, chevrons, undo/redo, skip, progress, indent. Never mirror: checks, play/pause/record, device rotation, clocks, search, terminal, keys, volume.
- No logo is defined yet; set the name as "Jidr" in Geist Bold with "جذر" in Noto Kufi Arabic SemiBold in `jidr-color-root`.

## Numbers

- Disk, downloads and image sizes: decimal GB (10⁹) with one decimal, matching Finder and About This Mac › Storage ("6.8 GB"). RAM: whole GB meaning 2³⁰ bytes, the convention every RAM spec and the emulator's `hw.ramSize` use ("4 GB"). CLI `--json`: raw bytes.
- Percent without a space (64%); number, no-break space, unit (40 s, 12 m, 2 h 14 m); speed with × (2×).
- Coordinates: signed decimal degrees, latitude first, 4 decimals shown and 6 accepted ("32.8954, 13.1802"); altitude in m, accuracy ± m, bearing in °.
- Logcat timestamps keep logcat's threadtime format (`10-13 14:02:07.318`). UI timestamps: relative day + 24 h time in the Mac's locale ("Today 14:02"), absolute after 7 days.
- Digits stay Western (0–9) in Arabic. Serials, ports, sizes and package names are isolated LTR.

## RTL

Layouts mirror cleanly for the Arabic build: the window (traffic lights included, as macOS does), sidebar, inspector, device toolbar and widget panel swap sides. Logcat, code, CLI, kbd hints and the Android screen do not mirror.
