import { latestRelease } from "../scripts/release.mjs";

// What the site shows and where it links. Everything that changes between the
// coming-soon page, launch day and later releases lives here.

// "soon" is the 11a coming-soon page (Star and Sponsor, no download); "launch"
// is the 11b page with the download, agent, CLI and docs sections.
export const phase: "soon" | "launch" = "launch";

// Public links (D1, D25, D26). `sponsors` stays null
// until a GitHub Sponsors profile exists: the Sponsor buttons and section are
// hidden while it's null.
const repo = "https://github.com/AbdurahmanAlmehdi/jidr";
const sponsors: string | null = null;

// The release the Download buttons point at: the newest complete GitHub
// release, looked up when the site builds (scripts/release.mjs). A failed
// lookup fails the production build, so the last good deploy stays up; `astro
// dev` falls back to placeholders.
async function currentRelease() {
  try {
    return await latestRelease(repo);
  } catch (e) {
    if (import.meta.env.PROD) throw e;
    console.warn(`release lookup failed, using placeholders: ${e}`);
    return { version: "0.0.0", sizeMb: "?", sha256: "(offline)", dmg: `${repo}/releases/latest` };
  }
}

export const release = {
  ...(await currentRelease()),
  // The cask in AbdurahmanAlmehdi/homebrew-tap, which release.yml keeps up to
  // date; shows the brew install and upgrade lines.
  homebrew: true,
};

export const links = {
  repo,
  sponsors,
  releases: `${repo}/releases`,
  dmg: release.dmg,
  doc: (path: string) => `${repo}/blob/main/${path}`,
};
