import { defineConfig } from "astro/config";
import vue from "@astrojs/vue";

// Static output for Cloudflare Pages (D16). Vue only powers interactive islands.
export default defineConfig({
  site: "https://jidr.ly",
  integrations: [vue()],
  build: { inlineStylesheets: "always" },
});
