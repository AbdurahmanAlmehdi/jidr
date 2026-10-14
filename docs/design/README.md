# Jidr design

Direction **Workbench**: one IDE-like window, dark-first, soft cards, devices open as tabs. Read `system/README.md` first: it holds the voice, colour, status, type, motion, icon and RTL rules.

| What | Where | Source artifact |
|---|---|---|
| Tokens (W3C DTCG, dark + light) | `tokens.json` | [Design system](https://claude.ai/artifact/KUTTdZ7wH6jyk4B7QFZ1kP) |
| Design-system rules | `system/README.md` | same |
| Component specs + previews (50) | `system/components/<Name>/README.md`, `preview.html`, `bundle.css` | same |
| Reference page (all tokens rendered, contrast notes) | `system-reference.html` | [Design system reference](https://claude.ai/artifact/8zj2Fes5rLJJ7qZaXernN1) |
| Final screens | `screens/*.dc.html` (+ `jidr.css`, `canvas.json`) | [Final designs](https://claude.ai/artifact/NSpzNW9nKpEWfE5fWuZx5j) |
| Product brief | `PRODUCT.md` | — |
| Brand (icon, symbol, web set) | `../../assets/brand/` | — |

The `.dc.html` files are design-canvas sources. They render in the artifact; open the links above to see them. Locally they're readable markup with the real copy, states and sample data.

## Chosen concepts → plan phases

| Screen file | What it shows | Phase |
|---|---|---|
| `Main.dc.html` | **1A (chosen):** launcher with sidebar device list + device detail; first run and populated, dark/light, 1280 check, optional live CPU/RAM (not in daemon data, so skip) | 7a, 7b |
| `Launcher-B.dc.html` | 1B card grid (not chosen; reference only) | — |
| `Setup.dc.html` | first-run window: find SDK → Google license (scroll to accept) → download → first device | 7b |
| `Create.dc.html` | create-device sheet (profile, Android version, root-mode cards) | 7b, 9e |
| `Device-Concepts.dc.html` | 4A docked widget panel vs 4B floating inspector | — |
| `Device.dc.html` | **4A (chosen):** device window for "Pixel-ish 6.1": toolbar, docked widgets (GPS, battery, …), logcat, APK drop, pixel-perfect | 7c, 8a, 8b, 10a |
| `Root.dc.html` | root flow: steps, success, failure + revert, Play Store beta | 9e |
| `Snapshots.dc.html` | snapshot manager with root-mode tags and the cross-mode refusal | 10b |
| `Settings.dc.html` | SDK location and disk use, cache, MCP setup, map tiles, default root mode, licences | 7b, 5b |
| `Landing-A.dc.html` | **8A (chosen):** headline over the app | 11a |
| `Landing-B.dc.html` | 8B split, terminal-led (not chosen) | — |
| `Landing.dc.html`, `Landing-Soon-390.dc.html` | coming-soon page, 1440 + 390 | 11a |
| `Landing-Launch.dc.html`, `Landing-Launch-390.dc.html` | launch page, 1440 + 390 | 11b |
| `Landing-AR.dc.html`, `Landing-AR-390.dc.html` | Arabic RTL hero | 11b |

## Implementation rules (D41)

- **One source:** `tokens.json`. Never hand-copy hex values.
  - The **Slint** theme (`crates/jidr-ui/ui/theme.slint`, `export global Theme`) is generated from it by `crates/jidr-ui/build.rs` (Stream B).
  - The **site's** `site/src/styles/tokens.css` is generated from it by a script in `site/` (Stream C).
  - Naming: CSS `--jidr-color-surface-raised` → Slint `Theme.surface-raised`. A leading `color-` is dropped in Slint; every other path stays the same.
- **Fonts:** Geist, Geist Mono and Noto Kufi Arabic (all OFL-1.1). Bundle them in the app (`apps/jidr-gui/assets/fonts/`, with licences) because Slint can't fetch web fonts. Self-host them on the site rather than loading Google Fonts.
- **Icons:** Lucide only (ISC). SVGs are loaded with `@image-url` and tinted with `colorize`. Credit Lucide in About › Licenses and the site footer.
- **Brand vs tokens:** the app icon gradient (`#2A88FA → #1267E0`, `assets/brand/`) is the approved icon and stays as is. Inside the UI, use tokens only: accent `jidr-color-accent-*` for actions, root violet `jidr-color-root` for root only. The system README's line "No logo is defined yet" predates the brand assets; the logo now exists in `assets/brand/`. The Arabic wordmark جذر uses `jidr-color-root`.
- Slint constraints the system already respects: no dashed borders, one shadow per element, no web fonts.
