#!/usr/bin/env bash
# Developer ID signing and notarization (Phase 12b, D32). `cargo xtask release`
# runs it when JIDR_SIGNING_IDENTITY is set: `app` before the DMG is built,
# `dmg` after.
#
#   packaging/macos/sign.sh app Jidr.app        # hardened runtime, inside-out
#   packaging/macos/sign.sh dmg Jidr-X-arm64.dmg # sign, notarize, staple
#
# Environment:
#   JIDR_SIGNING_IDENTITY   "Developer ID Application: <Name> (<TEAMID>)"
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD   notarytool credentials
#                           (an app-specific password, never the account password)
set -euo pipefail

die() { echo "sign.sh: $*" >&2; exit 1; }
need() { [ -n "${!1:-}" ] || die "$1 is not set"; }

[ $# -eq 2 ] || die "usage: sign.sh app <Jidr.app> | sign.sh dmg <file.dmg>"
kind=$1 target=$2
[ -e "$target" ] || die "$target does not exist"
need JIDR_SIGNING_IDENTITY

sign() {
    codesign --force --timestamp --options runtime \
        --sign "$JIDR_SIGNING_IDENTITY" "$@"
}

case "$kind" in
    app)
        # Inside-out: nested executables first, then the bundle. No --deep.
        sign "$target/Contents/Helpers/jidr"
        sign "$target/Contents/MacOS/Jidr"
        sign "$target"
        codesign --verify --strict --verbose=2 "$target"
        ;;
    dmg)
        need APPLE_ID
        need APPLE_TEAM_ID
        need APPLE_APP_PASSWORD
        sign "$target"
        xcrun notarytool submit "$target" \
            --apple-id "$APPLE_ID" \
            --team-id "$APPLE_TEAM_ID" \
            --password "$APPLE_APP_PASSWORD" \
            --wait
        xcrun stapler staple "$target"
        ;;
    *)
        die "unknown kind '$kind' (app or dmg)"
        ;;
esac
echo "sign.sh: $kind signed: $target"
