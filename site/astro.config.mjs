import { defineConfig } from "astro/config";

// Static output for Cloudflare Pages (D16): English at /, Arabic at /ar/ (D33).
// The only script is the Copy button's, a few lines Astro bundles; Vue
// islands come in when a page needs state.
export default defineConfig({
  site: "https://jidr.ly",
  build: { inlineStylesheets: "always" },
  i18n: {
    locales: ["en", "ar"],
    defaultLocale: "en",
    routing: { prefixDefaultLocale: false },
  },
});
