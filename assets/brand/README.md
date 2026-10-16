# Jidr brand assets

The symbol is a root (جذر) that grows into a phone outline. Everything here is generated from `source/appLogoOnPaper.jpg` by `build.sh`. Don't hand-edit outputs; change `build.sh` and re-run it.

```bash
brew install potrace librsvg imagemagick   # once
assets/brand/build.sh
```

## Colours

| Token | Hex | Use |
|---|---|---|
| Brand blue (top) | `#2A88FA` | icon gradient start |
| Brand blue (bottom) | `#1267E0` | icon gradient end; favicon symbol |
| Lilac | `#B9A6F7` | Arabic wordmark جذر on dark |
| Ink | `#101010` | social card background |

These were sampled from the approved board (`source/brand-board.jpg`). When the design system's `tokens.json` lands in `docs/design/`, it is the source of truth; update the colours in `build.sh` to match and re-run.

## Files

| File | What | Used by |
|---|---|---|
| `symbol.svg` | the symbol, `fill="currentColor"` | everything; tint it |
| `symbol-bold.svg` | thicker strokes for ≤32 px | small icons, favicon, menu bar |
| `app-icon.svg` / `app-icon-1024.png` | macOS icon master (1024 canvas, 824 squircle, shadow) | `.icns`, website, docs |
| `app-icon-small.svg` | 16/32 px cut (no shadow, larger body) | `.icns` small sizes, favicon |
| `avatar.svg` / `avatar-1024.png` | circular avatar | GitHub org/repo, social profiles |
| `social-card.svg` / `.png` | 1200×630 preview card | Open Graph / Twitter card |
| `web/` | `favicon.svg`, `favicon.ico`, `favicon-32.png`, `apple-touch-icon.png`, `og-image.png`, `avatar-512.png`, `symbol.svg` | **site** (copied into `site/public/`) |
| `../../apps/jidr-gui/assets/macos/Jidr.icns` | app bundle icon (16–1024, @1x/@2x) | Phase 12a bundle (`CFBundleIconFile`) |
| `../../apps/jidr-gui/assets/macos/AppIcon-1024.png` | master PNG | DMG background / About |
| `../../apps/jidr-gui/assets/menubar/MenuBarTemplate{,@2x}.png` | 18 pt template image (black + alpha; macOS tints it) | menu bar extra (post-MVP) |
| `../../apps/jidr-gui/assets/icons/symbol.svg` | in-app symbol; Slint renders SVG natively, tint with `colorize` | Phase 7a/7b (title bar, empty states, About) |
| `../../apps/jidr-gui/assets/icons/app-icon-256.png` | window/About icon | Phase 7a |

## Rules

- Never recolour the app icon gradient outside the tokens above. Tint `symbol.svg` with text or accent tokens in-app.
- Keep clear space around the symbol of at least ¼ of its width.
- No Google, Android or Genymotion marks next to the symbol.
- Ownership: this folder is shared and read-only for the work streams. Only `build.sh` changes it.
