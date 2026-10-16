# macOS packaging

`cargo xtask release` (in `xtask/src/release.rs`) builds `Jidr.app` and `Jidr-<version>-arm64.dmg` from this folder.

| File | What |
|---|---|
| `Info.plist.in` | bundle metadata; `@VERSION@` becomes the workspace version |
| `sign.sh` | 12b: Developer ID signing (hardened runtime, inside-out) and DMG notarization + staple. Prepared, not wired in. |
| `verify.sh` | 12b validation: Gatekeeper, hardened runtime and the stapled ticket. Fails on today's ad-hoc builds, as expected. |
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

Signing (D32): until the Developer ID signing secrets are configured, the app is ad-hoc signed. Users open it the first time with right-click → Open. Phase 12b adds hardened-runtime signing, notarization and the Homebrew cask.

The scan fails the build if the bundle or the DMG contains Google SDK binaries, system images or Magisk. Jidr downloads those on the user's machine, never ships them.
