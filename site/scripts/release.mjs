// The release jidr.ly's Download buttons point at, read from GitHub when the
// site builds: the newest published release whose DMG and SHA256SUMS.txt are
// already uploaded. release.yml uploads them after tagging, so a build in the
// middle of a release still shows the last complete one, and the release's
// deploy hook rebuilds the site once the new files are there.
//
// Set GITHUB_TOKEN (read-only) where the site builds to stay clear of the
// API's limit of 60 unauthenticated requests an hour per IP.

/** The newest non-draft, non-prerelease release with its DMG and checksums. */
export function pickRelease(releases) {
  for (const r of releases) {
    if (r.draft || r.prerelease) continue;
    const version = r.tag_name.replace(/^v/, "");
    const dmg = r.assets.find((a) => a.name === `Jidr-${version}-arm64.dmg`);
    const sums = r.assets.find((a) => a.name === "SHA256SUMS.txt");
    if (dmg && sums) return { version, dmg, sums };
  }
  return null;
}

/** The SHA-256 of `name` in a `shasum -a 256` listing. */
export function shaFor(sums, name) {
  for (const line of sums.split("\n")) {
    const [hash, file] = line.trim().split(/\s+\*?/);
    if (file === name && /^[0-9a-f]{64}$/.test(hash)) return hash;
  }
  return null;
}

/** Bytes as decimal megabytes, as Finder and Homebrew show them. */
export function sizeMb(bytes) {
  return (bytes / 1e6).toFixed(1);
}

/** `{ version, sizeMb, sha256, dmg }` for `https://github.com/<owner>/<repo>`. */
export async function latestRelease(repo, { fetch = globalThis.fetch, token = process.env.GITHUB_TOKEN } = {}) {
  const headers = { Accept: "application/vnd.github+json", "User-Agent": "jidr.ly-build" };
  if (token) headers.Authorization = `Bearer ${token}`;
  const api = `https://api.github.com/repos${new URL(repo).pathname}/releases?per_page=10`;
  const res = await fetch(api, { headers });
  if (!res.ok) throw new Error(`GitHub releases: HTTP ${res.status}`);
  const picked = pickRelease(await res.json());
  if (!picked) throw new Error("no published release has its DMG and SHA256SUMS.txt yet");
  const sums = await fetch(picked.sums.browser_download_url);
  if (!sums.ok) throw new Error(`SHA256SUMS.txt: HTTP ${sums.status}`);
  const sha256 = shaFor(await sums.text(), picked.dmg.name);
  if (!sha256) throw new Error(`SHA256SUMS.txt has no line for ${picked.dmg.name}`);
  return {
    version: picked.version,
    sizeMb: sizeMb(picked.dmg.size),
    sha256,
    dmg: picked.dmg.browser_download_url,
  };
}
