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
#   NOTARY_KEY_PATH         App Store Connect API key (.p8) for notarytool
#   NOTARY_KEY_ID           its key ID
#   NOTARY_ISSUER_ID        its issuer ID
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
        need NOTARY_KEY_PATH
        need NOTARY_KEY_ID
        need NOTARY_ISSUER_ID
        [ -f "$NOTARY_KEY_PATH" ] || die "NOTARY_KEY_PATH $NOTARY_KEY_PATH does not exist"
        sign "$target"
        result=$(xcrun notarytool submit "$target" \
            --key "$NOTARY_KEY_PATH" \
            --key-id "$NOTARY_KEY_ID" \
            --issuer "$NOTARY_ISSUER_ID" \
            --wait 2>&1) || die "notarytool failed: $result"
        echo "$result"
        # Check Apple's verdict, not only notarytool's exit code.
        grep -q "status: Accepted" <<<"$result" \
            || die "notarization was not accepted; run 'xcrun notarytool log <id>' with the same key for details"
        xcrun stapler staple "$target"
        ;;
    *)
        die "unknown kind '$kind' (app or dmg)"
        ;;
esac
echo "sign.sh: $kind signed: $target"
