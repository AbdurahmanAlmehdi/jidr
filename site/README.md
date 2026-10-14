# jidr.ly

The Jidr landing page: Astro (static) for Cloudflare Pages (D16), built to the Workbench design (D41). English at `/`, Arabic (RTL) at `/ar/` (D33). Not deployed yet; Abdurahman connects the domains.

```bash
npm ci
npm run dev          # http://localhost:4321 (generates the styles first)
npm run build        # → dist/ (generates the styles first)
npm test             # tests for the style generator
npm run lighthouse   # Lighthouse CI on dist/ (performance and accessibility ≥ 95)
```

- **Design source:** `docs/design/screens/Landing.dc.html` (concept 8A) with its `Landing-Launch*`, `Landing-AR*` and `Landing-Soon-390` frames. Each section in `src/components/` says which part of it it ports.
- **Phase:** `phase` in `src/config.ts`. `"launch"` (the default) shows the download, agent/MCP, CI snippet and docs sections; `"soon"` is the 11a coming-soon page (Star and Sponsor, no download).
- **Copy:** `src/i18n.ts` holds every string in English and Arabic; components read `Astro.currentLocale`. What depicts the English-only app (the product shot, the Rooted badge, the agent transcript, commands) stays English on both pages.
- **Before launch:** in `src/config.ts`, set `release.version`, `sizeMb` and `sha256` from the published release's `SHA256SUMS.txt`, and `homebrew: true` once the cask exists (Phase 12b). The Download buttons link to `Jidr-<version>-arm64.dmg` on the `v<version>` release.
- **Styles are generated, never hand-edited** (`scripts/build-styles.mjs`, run before `dev` and `build`; outputs are git-ignored):
  - `src/styles/tokens.css`: every token in `docs/design/tokens.json` as a `--jidr-*` custom property, dark and light. Until gap G13 is fixed, the Arabic landing type sizes (`--jidr-type-web-*-ar`) come from the token block of `docs/design/screens/jidr.css`, with a build warning; they drop out once `tokens.json` has them.
  - `src/styles/workbench.css`: the component rules of `docs/design/screens/jidr.css`, pruned to the classes the `.astro` files use. Use a class in a `class="…"` attribute and its rules come along.
  - `src/styles/site.css`: the few page-level rules the design canvas got from its host page, tokens only.
- **Fonts:** Geist, Geist Mono and Noto Kufi Arabic (OFL-1.1), self-hosted from `@fontsource`, only the weights and subsets in use. `src/layouts/Base.astro` has the faces both pages need and preloads each locale's hero faces; `src/pages/ar/index.astro` adds the Kufi faces only the Arabic page uses. Arabic headings use 600, the Arabic type group's weight, so no bold Kufi face loads.
- **Brand assets:** `public/{favicon.svg,favicon.ico,favicon-32.png,apple-touch-icon.png,og-image.png,symbol.svg}` are copies of `assets/brand/web/*` (D39). Re-copy them when `assets/brand/build.sh` changes.
- **Public links:** `src/config.ts` (the GitHub org and Sponsors profile don't exist yet).
- **Rules:** no email capture or analytics (D17, D34); no Google, Android or Genymotion logos; the Genymotion comparison is text only.
