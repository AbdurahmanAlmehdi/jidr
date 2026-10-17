// The sitemap: one entry per language, each listing the others as
// alternates (hreflang), so search engines pair /ar/ with /.
import type { APIRoute } from "astro";
import { locales, strings } from "../i18n";

export const GET: APIRoute = ({ site }) => {
  const url = (path: string) => new URL(path, site).href;
  const alternates = [
    ...locales.map((l) => `<xhtml:link rel="alternate" hreflang="${l}" href="${url(strings(l).home)}"/>`),
    `<xhtml:link rel="alternate" hreflang="x-default" href="${url("/")}"/>`,
  ].join("");
  const entries = locales.map((l) => `<url><loc>${url(strings(l).home)}</loc>${alternates}</url>`).join("");
  const body = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">${entries}</urlset>\n`;
  return new Response(body, { headers: { "Content-Type": "application/xml; charset=utf-8" } });
};
