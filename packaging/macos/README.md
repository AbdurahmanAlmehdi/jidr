# macOS packaging

`cargo xtask release` (in `xtask/src/release.rs`) builds `Jidr.app` and `Jidr-<version>-arm64.dmg` from this folder.

| File | What |
|---|---|
| `Info.plist.in` | bundle metadata; `@VERSION@` becomes the workspace version |
| `sign.sh` | Developer ID signing (hardened runtime, inside-out) and DMG notarization + staple; `cargo xtask release` calls it when `JIDR_SIGNING_IDENTITY` is set |
| `verify.sh` | Gatekeeper, hardened runtime and the stapled ticket; the release workflow runs it on signed builds. Fails on ad-hoc builds, as expected |
| `../homebrew/jidr.rb` | 12b: cask template for the future tap |

Bundle layout:

```
Jidr.app/Contents/
  Info.plist  PkgInfo
  MacOS/Jidr            GUI (apps/jidr-gui, GPL-3.0)
  Helpers/jidr          CLI, daemon and MCP server (apps/jidr, Apache-2.0)
  Resources/Jidr.icns   from apps/jidr-gui/assets/macos (D39)
  Resources/licenses/   LICENSE-APACHE, LICENSE-GPL, NOTICE
```

The CLI lives in `Helpers/` because `MacOS/jidr` and `MacOS/Jidr` are the same path on the default case-insensitive APFS.

```bash
cargo xtask release --dry-run --scan    # local build, DMG, mount check, forbidden-content scan
```

## Releases and signing (D32)

Releases are cut by release-please: every merge to `main` updates a "chore: release vX.Y.Z" pull request (version from conventional commits, `CHANGELOG.md`). Merging it tags the version and publishes the GitHub Release; `.github/workflows/release.yml` then builds the DMG on macOS, signs and notarizes it, runs `verify.sh` and uploads the DMG and `SHA256SUMS.txt` to the release.

Repository secrets for signing (without them the DMG is ad-hoc signed and the run warns; users then open the app from System Settings › Privacy & Security › Open Anyway):

| Secret | Value |
|---|---|
| `APPLE_DEVELOPER_ID_CERT` | the Developer ID Application certificate and its private key, exported from Keychain Access as a `.p12`, base64-encoded (`base64 -i cert.p12`) |
| `APPLE_DEVELOPER_ID_CERT_PASSWORD` | the password chosen when exporting the `.p12` |
| `APPLE_ID` | the Apple ID used for notarization |
| `APPLE_TEAM_ID` | the 10-character team ID |
| `APPLE_APP_PASSWORD` | an app-specific password for that Apple ID (account.apple.com), never the account password |
| `RELEASE_PLEASE_TOKEN` | optional: a fine-grained token with contents and pull-requests write, so the release PR runs CI |

To sign locally: `JIDR_SIGNING_IDENTITY="Developer ID Application: Name (TEAMID)" APPLE_ID=… APPLE_TEAM_ID=… APPLE_APP_PASSWORD=… cargo xtask release --scan`. The Homebrew cask (`packaging/homebrew/jidr.rb`) is still a template.

The scan fails the build if the bundle or the DMG contains Google SDK binaries, system images or Magisk. Jidr downloads those on the user's machine, never ships them.
