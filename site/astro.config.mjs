import { defineConfig } from "astro/config";

// Static output for Cloudflare Pages (D16). Vue islands come back when a page
// needs interaction (11b's copy buttons); the coming-soon page has none.
export default defineConfig({
  site: "https://jidr.ly",
  build: { inlineStylesheets: "always" },
});
