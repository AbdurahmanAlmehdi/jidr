# macOS packaging

`cargo xtask release` (in `xtask/src/release.rs`) builds `Jidr.app`, `Jidr-<version>-arm64.dmg` and the MCP bundle `jidr-mcp-<version>-darwin-arm64.mcpb` from this folder and `../mcp`.

| File | What |
|---|---|
| `Info.plist.in` | bundle metadata; `@VERSION@` becomes the workspace version |
| `sign.sh` | Developer ID signing (hardened runtime, inside-out) and DMG notarization + staple; `cargo xtask release` calls it when `JIDR_SIGNING_IDENTITY` is set |
| `verify.sh` | Gatekeeper, hardened runtime and the stapled ticket; the release workflow runs it on signed builds. Fails on ad-hoc builds, as expected |
| `../homebrew/jidr.rb` | cask template; `release.yml` fills in the version and checksum and pushes it to `AbdurahmanAlmehdi/homebrew-tap` |
| `../mcp/manifest.json` | MCPB manifest (`@VERSION@`); the bundle holds it, the signed `jidr` as `server/jidr`, the icon and the licences. `jidr-mcp`'s tests check its tool list |
| `../../server.json` | MCP Registry entry; `release.yml` points it at the release's `.mcpb` and publishes it |

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

Releases are cut by release-please: every merge to `main` updates a "chore: release vX.Y.Z" pull request (version from conventional commits, `CHANGELOG.md`). Merging it tags the version and publishes the GitHub Release; `.github/workflows/release.yml` then builds the DMG and the `.mcpb` on macOS, signs and notarizes them, runs `verify.sh` and uploads both with `SHA256SUMS.txt`. For a signed build it then pushes `Casks/jidr.rb` to the tap (`brew install --cask abdurahmanalmehdi/tap/jidr`) and publishes `server.json` to the [MCP Registry](https://registry.modelcontextprotocol.io) with GitHub OIDC.

Repository secrets for signing (without them the DMG is ad-hoc signed and the run warns; users then open the app from System Settings › Privacy & Security › Open Anyway):

| Secret | Value |
|---|---|
| `MAC_DEVELOPER_ID_P12_BASE64` | the Developer ID Application certificate and its private key, exported from Keychain Access as a `.p12`, base64-encoded |
| `MAC_DEVELOPER_ID_P12_PASSWORD` | the password chosen when exporting the `.p12` |
| `MAC_NOTARY_KEY_P8_BASE64` | an App Store Connect API key (`.p8`, from App Store Connect › Users and Access › Integrations), base64-encoded |
| `MAC_NOTARY_KEY_ID` | that key's ID |
| `MAC_NOTARY_ISSUER_ID` | that key's issuer ID |
| `RELEASE_PLEASE_TOKEN` | optional: a fine-grained token with contents and pull-requests write, so the release PR runs CI |
| `HOMEBREW_TAP_TOKEN` | a fine-grained token with contents write on `AbdurahmanAlmehdi/homebrew-tap` only; without it the cask isn't updated and the run warns |

Set them without printing anything, for example `base64 -i developer-id.p12 | gh secret set MAC_DEVELOPER_ID_P12_BASE64`, and check the file exists first: if `base64` fails, `gh` stores an empty secret.

To sign and notarize locally: `JIDR_SIGNING_IDENTITY="Developer ID Application: Name (TEAMID)" NOTARY_KEY_PATH=path/to/AuthKey.p8 NOTARY_KEY_ID=… NOTARY_ISSUER_ID=… cargo xtask release --scan`.

The scan fails the build if the bundle or the DMG contains Google SDK binaries, system images or Magisk. Jidr downloads those on the user's machine, never ships them.
