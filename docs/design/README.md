# Jidr design

Direction **Workbench**: one IDE-like window, dark-first, soft cards, devices open as tabs. Read `system/README.md` first: it holds the voice, colour, status, type, motion, icon and RTL rules.

| What | Where |
|---|---|
| Tokens (W3C DTCG, dark and light) | `tokens.json` |
| Design-system rules | `system/README.md` |
| Component specs (50) | `system/components/<Name>/README.md` |
| Component CSS (the app's reduced-motion overrides come from it) | `system/components/bundle.css` |
| Workbench component CSS (the site's styles are pruned from it) | `jidr.css` |
| Product brief | `PRODUCT.md` |
| Brand (icon, symbol, web set) | `../../assets/brand/` |

Chosen concepts: launcher **1A** (sidebar device list and device detail), device window **4A** (docked widget panel), landing page **8A** (headline over the app).

## Implementation rules (D41)

- **One source:** `tokens.json`. Never hand-copy hex values.
  - The **Slint** theme (`crates/jidr-ui/ui/theme.slint`, `export global Theme`) is generated from it by `crates/jidr-ui/build.rs`.
  - The **site's** `site/src/styles/tokens.css` is generated from it by a script in `site/`.
  - Naming: CSS `--jidr-color-surface-raised` → Slint `Theme.surface-raised`. A leading `color-` is dropped in Slint; every other path stays the same.
- **Fonts:** Geist, Geist Mono and Noto Kufi Arabic (all OFL-1.1). Bundle them in the app (`apps/jidr-gui/assets/fonts/`, with licences) because Slint can't fetch web fonts. Self-host them on the site rather than loading Google Fonts.
- **Icons:** Lucide only (ISC). SVGs are loaded with `@image-url` and tinted with `colorize`. Credit Lucide in About › Licenses and the site footer.
- **Brand vs tokens:** the app icon gradient (`#2A88FA → #1267E0`, `assets/brand/`) is the approved icon and stays as is. Inside the UI, use tokens only: accent `jidr-color-accent-*` for actions, root violet `jidr-color-root` for root only. The Arabic wordmark جذر uses `jidr-color-root`.
- Slint constraints the system already respects: no dashed borders, one shadow per element, no web fonts.
