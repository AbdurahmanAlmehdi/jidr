# jidr.ly

The Jidr landing page: Astro (static) with Vue islands only where interactive (D16), deployed to Cloudflare Pages. Not deployed yet; Abdurahman connects the domains.

```bash
npm ci
npm run dev          # http://localhost:4321
npm run build        # → dist/
npm run lighthouse   # Lighthouse CI on dist/ (performance and accessibility ≥ 95)
```

- Design tokens: `src/styles/tokens.css`, the only place colours, type, spacing and radii are defined. They are placeholders named to match the future `docs/design/tokens.json` (`--jidr-color-surface-raised` ↔ `Theme.surface-raised`), so restyling is one file.
- Public links: `src/config.ts` (the GitHub org and Sponsors profile don't exist yet).
- Rules: no email capture or analytics (D17, D34); no Google, Android or Genymotion logos; text-only feature comparison.
