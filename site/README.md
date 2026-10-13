# jidr.ly

The Jidr landing page: Astro (static) with Vue islands only where interactive (D16), deployed to Cloudflare Pages. Not deployed yet; Abdurahman connects the domains.

```bash
npm ci
npm run dev          # http://localhost:4321
npm run build        # → dist/
npm run lighthouse   # Lighthouse CI on dist/ (performance and accessibility ≥ 95)
```

- Design tokens: `src/styles/tokens.css`, the only place colours, type, spacing and radii are defined. Accent and wordmark colours follow `assets/brand/README.md`; the rest are placeholders named to match the future `docs/design/tokens.json` (`--jidr-color-surface-raised` ↔ `Theme.surface-raised`), so restyling is one file.
- Brand assets: `public/{favicon.svg,favicon.ico,favicon-32.png,apple-touch-icon.png,og-image.png,symbol.svg}` are copies of `assets/brand/web/*` (D39). Re-copy them when `assets/brand/build.sh` changes; don't edit them here.
- Public links: `src/config.ts` (the GitHub org and Sponsors profile don't exist yet).
- Rules: no email capture or analytics (D17, D34); no Google, Android or Genymotion logos; text-only feature comparison.
