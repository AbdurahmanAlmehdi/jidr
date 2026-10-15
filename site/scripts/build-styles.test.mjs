import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { parseCss, print, prune, tokensCss } from "./build-styles.mjs";

const here = dirname(fileURLToPath(import.meta.url));

const tokens = {
  type: {
    families: { ui: "Geist, sans-serif", mono: "\"Geist Mono\", monospace", arabic: "\"Noto Kufi Arabic\"" },
    groups: [
      {
        name: "App",
        family: "ui",
        styles: [
          { name: "type-hero", fontSize: "32px", lineHeight: "38px", fontWeight: 600, letterSpacing: "-0.8px" },
          { name: "type-metric", fontSize: "13px", lineHeight: "18px", fontWeight: 500, letterSpacing: "0px", family: "mono" },
        ],
      },
      {
        name: "Arabic",
        family: "arabic",
        styles: [{ name: "type-hero-ar", fontSize: "30px", lineHeight: "52px", fontWeight: 600, letterSpacing: "0px" }],
      },
    ],
  },
  color: {
    tokens: [
      { name: "jidr-color-accent-text", value: { dark: "#8fb0ff", light: "#1d4ed8" } },
      { name: "jidr-color-text-link", value: { dark: "{jidr-color-accent-text}", light: "{jidr-color-accent-text}" } },
    ],
  },
  shadow: { tokens: [{ name: "jidr-elevation-3-shadow", value: { dark: "0px 16px 40px 0px #00000000", light: "0px 16px 40px 0px #12151A33" } }] },
  spacing: { tokens: [{ name: "jidr-space-lg", value: "16px" }] },
  duration: {
    tokens: [
      { name: "jidr-motion-hover-duration", value: "120ms", usage: "Colour change. It stays under Reduce Motion." },
      { name: "jidr-motion-panel-duration", value: "240ms", usage: "Panel width. Reduced: instant." },
      { name: "jidr-motion-popover-duration", value: "160ms", usage: "Fade + rise. Reduced: fade only." },
      { name: "jidr-motion-popover-distance", value: "4px", usage: "Fade + rise. Reduced: fade only." },
    ],
  },
};

test("expands type roles with Arabic variants and metric features", () => {
  const css = tokensCss(tokens);
  assert.match(css, /--jidr-font-family-ui: Geist, sans-serif;/);
  assert.match(css, /--jidr-type-hero-family: var\(--jidr-font-family-ui\);/);
  assert.match(css, /--jidr-type-hero-size-ar: 30px;/);
  assert.match(css, /--jidr-type-hero-line-height-ar: 52px;/);
  assert.match(css, /--jidr-type-metric-family: var\(--jidr-font-family-mono\);/);
  assert.match(css, /--jidr-type-metric-features: "tnum";/);
  assert.match(css, /--jidr-font-weight-semibold: 600;/);
  assert.doesNotMatch(css, /type-hero-ar-/);
});

test("resolves DTCG aliases and puts themed values in theme blocks", () => {
  // Regression: aliases were emitted as `{jidr-…}` instead of var(--jidr-…).
  const css = tokensCss(tokens);
  assert.doesNotMatch(css, /\{jidr-/);
  assert.match(css, /--jidr-color-text-link: var\(--jidr-color-accent-text\);/);
  const dark = css.split(":root,\n.jidr-dark {")[1].split("\n}")[0];
  assert.match(dark, /color-scheme: dark/);
  assert.match(dark, /--jidr-color-accent-text: #8fb0ff;/);
  assert.match(css, /:root\[data-theme="light"\],\n\.jidr-light \{\n  color-scheme: light;/);
});

test("splits shadows into colour, y and blur", () => {
  const css = tokensCss(tokens);
  assert.match(css, /--jidr-elevation-3-shadow-color: #12151A33;/);
  assert.match(css, /--jidr-elevation-3-shadow-y: 16px;/);
  assert.match(css, /--jidr-elevation-3-shadow-blur: 40px;/);
});

test("reduced motion follows each token's own note", () => {
  const css = tokensCss(tokens);
  const reduced = css.split("@media (prefers-reduced-motion: reduce)")[1];
  assert.match(reduced, /--jidr-motion-panel-duration: 0ms;/);
  assert.match(reduced, /--jidr-motion-popover-distance: 0px;/);
  assert.doesNotMatch(reduced, /popover-duration|hover-duration/);
});

test("prunes rules to the used classes, keeping at-rules and keyframes in use", () => {
  const css = `
    *{box-sizing:border-box}
    .btn,.btn.is-hover{color:red}
    .unused{color:blue}
    .site .lp-h2{font-size:30px}
    @container (max-width: 760px){.wrap{padding:16px}.gone{x:y}}
    @keyframes spin{to{transform:rotate(1turn)}}
    @keyframes unused-anim{to{opacity:0}}
    .spinner{animation:spin 1s}
  `;
  const out = print(prune(parseCss(css), new Set(["btn", "site", "lp-h2", "wrap"])));
  assert.match(out, /\*\{box-sizing:border-box\}/);
  assert.match(out, /^\.btn\{color:red\}$/m);
  assert.match(out, /\.site \.lp-h2/);
  assert.match(out, /@container \(max-width: 760px\)\{\n\.wrap\{padding:16px\}\n\}/);
  assert.doesNotMatch(out, /unused|gone|is-hover|spinner|@keyframes/);
});

test("the real tokens resolve every variable the design's component CSS uses", () => {
  const design = join(here, "../../docs/design");
  const jidrCss = readFileSync(join(design, "screens/jidr.css"), "utf8");
  const tokens = tokensCss(JSON.parse(readFileSync(join(design, "tokens.json"), "utf8")));
  const defined = new Set([...tokens.matchAll(/(--jidr-[\w-]+):/g)].map((m) => m[1]));
  const comp = jidrCss.split("page chrome */")[1];
  const used = new Set([...comp.matchAll(/var\((--jidr-[\w-]+)/g)].map((m) => m[1]));
  assert.deepEqual([...used].filter((v) => !defined.has(v)), []);
});

// Regression (G13): tokens.json had no Arabic landing sizes, so the RTL hero,
// lede and section titles pointed at undefined variables. The design's token
// block and tokens.json must now agree on every Arabic size.
test("tokens.json carries every Arabic type size jidr.css's token block declares", () => {
  const design = join(here, "../../docs/design");
  const jidrCss = readFileSync(join(design, "screens/jidr.css"), "utf8");
  const tokens = tokensCss(JSON.parse(readFileSync(join(design, "tokens.json"), "utf8")));
  const decls = (css) =>
    new Map([...(/:root\s*\{([^}]*)\}/.exec(css)?.[1] ?? "").matchAll(/(--jidr-type-[\w-]+-ar):\s*([^;]+);/g)]
      .map(([, name, value]) => [name, value.trim()]));
  const want = decls(jidrCss);
  const ours = decls(tokens);
  assert.ok(want.size >= 26, `jidr.css declares ${want.size} Arabic sizes`);
  for (const [name, value] of want) assert.equal(ours.get(name), value, name);
});
