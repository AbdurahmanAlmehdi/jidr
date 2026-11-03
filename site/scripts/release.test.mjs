import { test } from "node:test";
import assert from "node:assert/strict";

import { latestRelease, pickRelease, shaFor, sizeMb } from "./release.mjs";

const SHA_020 = "55a59a7e832ddbcbf6aef6875950ef7df3975a751a6040d90c5abdf5c3f1e924";
const SHA_010 = "13b49341c3ff2c860cfdfbd4bfcef4324e6524134f99d4f1fe119dc4a58fbac6";

const asset = (name, size = 1) => ({
  name,
  size,
  browser_download_url: `https://github.com/o/r/releases/download/x/${name}`,
});

// Newest first, as GET /repos/{owner}/{repo}/releases returns them.
const releases = [
  // Tagged a minute ago: release.yml hasn't uploaded the DMG yet.
  { tag_name: "v0.3.0", draft: false, prerelease: false, assets: [] },
  { tag_name: "v0.2.1-rc.1", draft: false, prerelease: true, assets: [asset("Jidr-0.2.1-rc.1-arm64.dmg"), asset("SHA256SUMS.txt")] },
  { tag_name: "v0.2.0", draft: false, prerelease: false, assets: [asset("Jidr-0.2.0-arm64.dmg", 21969767), asset("SHA256SUMS.txt")] },
  { tag_name: "v0.1.0", draft: false, prerelease: false, assets: [asset("Jidr-0.1.0-arm64.dmg"), asset("SHA256SUMS.txt")] },
];

const sums = `${SHA_020}  Jidr-0.2.0-arm64.dmg\n94977513a2c2944f5d968af70ce3df4b02dc39bf4a0a7226d8e6c5c237a49eab  jidr-mcp-0.2.0-darwin-arm64.mcpb\n`;

test("skips releases whose files aren't uploaded yet, drafts and prereleases", () => {
  assert.equal(pickRelease(releases)?.version, "0.2.0");
  assert.equal(pickRelease([releases[0]]), null);
});

test("finds the DMG's line in SHA256SUMS.txt", () => {
  assert.equal(shaFor(sums, "Jidr-0.2.0-arm64.dmg"), SHA_020);
  assert.equal(shaFor(`${SHA_010} *Jidr-0.1.0-arm64.dmg`, "Jidr-0.1.0-arm64.dmg"), SHA_010);
  assert.equal(shaFor(sums, "Jidr-9.9.9-arm64.dmg"), null);
});

test("sizes are decimal megabytes", () => {
  assert.equal(sizeMb(21969767), "22.0");
  assert.equal(sizeMb(21931184), "21.9");
});

test("reads the release and its checksum from GitHub", async () => {
  const seen = [];
  const fetch = async (url, init) => {
    seen.push({ url, auth: init?.headers?.Authorization });
    const body = url.endsWith("SHA256SUMS.txt") ? sums : JSON.stringify(releases);
    return new Response(body, { status: 200 });
  };
  const r = await latestRelease("https://github.com/o/r", { fetch, token: "t" });
  assert.deepEqual(r, {
    version: "0.2.0",
    sizeMb: "22.0",
    sha256: SHA_020,
    dmg: "https://github.com/o/r/releases/download/x/Jidr-0.2.0-arm64.dmg",
  });
  assert.equal(seen[0].url, "https://api.github.com/repos/o/r/releases?per_page=10");
  assert.equal(seen[0].auth, "Bearer t");
});

test("a failed lookup throws, so the build fails and the last deploy stays up", async () => {
  const fetch = async () => new Response("rate limited", { status: 403 });
  await assert.rejects(latestRelease("https://github.com/o/r", { fetch, token: "" }), /HTTP 403/);
});
