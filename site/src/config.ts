// What the site shows and where it links. Everything that changes between the
// coming-soon page, launch day and later releases lives here.

// "soon" is the 11a coming-soon page (Star and Sponsor, no download); "launch"
// is the 11b page with the download, agent, CLI and docs sections.
export const phase: "soon" | "launch" = "launch";

// Planned public links (D1, D25, D26). The org, repo and Sponsors profile don't
// exist yet; Abdurahman creates them before launch, then only these change.
const repo = "https://github.com/jidr-dev/jidr";

// The release the Download buttons point at. `xtask release` names the DMG
// `Jidr-<version>-arm64.dmg` and `release.yml` tags it `v<version>`. Fill in
// the size and the SHA-256 from the published release's SHA256SUMS.txt.
export const release = {
  version: "0.1.0",
  sizeMb: "[size]",
  sha256: "[checksum]",
  // The Homebrew cask ships with signing and notarization (Phase 12b).
  homebrew: false,
};

export const links = {
  repo,
  sponsors: "https://github.com/sponsors/jidr-dev",
  releases: `${repo}/releases`,
  dmg: `${repo}/releases/download/v${release.version}/Jidr-${release.version}-arm64.dmg`,
  doc: (path: string) => `${repo}/blob/main/${path}`,
};
