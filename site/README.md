# jidr.ly

The Jidr landing page: Astro (static) for Cloudflare Pages (D16), built to the Workbench design (D41). Not deployed yet; Abdurahman connects the domains.

```bash
npm ci
npm run dev          # http://localhost:4321 (generates the styles first)
npm run build        # → dist/ (generates the styles first)
npm test             # tests for the style generator
npm run lighthouse   # Lighthouse CI on dist/ (performance and accessibility ≥ 95)
```

- **Design source:** `docs/design/screens/Landing.dc.html` (concept 8A, phase "soon"). Each section in `src/components/` says which part of it it ports.
- **Styles are generated, never hand-edited** (`scripts/build-styles.mjs`, run before `dev` and `build`; outputs are git-ignored):
  - `src/styles/tokens.css`: every token in `docs/design/tokens.json` as a `--jidr-*` custom property, dark and light.
  - `src/styles/workbench.css`: the component rules of `docs/design/screens/jidr.css`, pruned to the classes the `.astro` files use. Use a class in a `class="…"` attribute and its rules come along.
  - `src/styles/site.css`: the few page-level rules the design canvas got from its host page, tokens only.
- **Fonts:** Geist, Geist Mono and Noto Kufi Arabic (OFL-1.1), self-hosted from `@fontsource`, only the weights and subsets in use, preloaded in `src/layouts/Base.astro`.
- **Brand assets:** `public/{favicon.svg,favicon.ico,favicon-32.png,apple-touch-icon.png,og-image.png,symbol.svg}` are copies of `assets/brand/web/*` (D39). Re-copy them when `assets/brand/build.sh` changes.
- **Public links:** `src/config.ts` (the GitHub org and Sponsors profile don't exist yet).
- **Rules:** no email capture or analytics (D17, D34); no Google, Android or Genymotion logos; the Genymotion comparison is text only.
