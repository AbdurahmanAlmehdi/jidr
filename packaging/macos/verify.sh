#!/usr/bin/env bash
# Gatekeeper checks for a signed, notarized release (Phase 12b validation).
#
#   packaging/macos/verify.sh <Jidr-X-arm64.dmg>
#
# Mounts the DMG read-only and checks the DMG and the app inside it. Exits
# non-zero for ad-hoc (unsigned, 12a) builds: that's expected until 12b.
set -euo pipefail

die() { echo "verify.sh: $*" >&2; exit 1; }
[ $# -eq 1 ] || die "usage: verify.sh <file.dmg>"
dmg=$1
[ -f "$dmg" ] || die "$dmg does not exist"

mnt=$(mktemp -d)
trap 'hdiutil detach "$mnt" >/dev/null 2>&1 || true; rmdir "$mnt" 2>/dev/null || true' EXIT
hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "$mnt" "$dmg" >/dev/null
app="$mnt/Jidr.app"
[ -d "$app" ] || die "no Jidr.app in $dmg"

echo "== DMG signature and ticket"
codesign --verify --strict --verbose=2 "$dmg"
xcrun stapler validate "$dmg"
spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"

echo "== Jidr.app"
codesign --verify --deep --strict --verbose=2 "$app"
codesign --display --verbose=2 "$app" 2>&1 | grep -E '^(Authority|TeamIdentifier|Runtime)'
codesign --display --verbose=2 "$app" 2>&1 | grep -q 'flags=.*runtime' \
    || die "Jidr.app is not signed with the hardened runtime"
spctl --assess --type execute --verbose=2 "$app"

echo "verify.sh: ok: $dmg is signed, notarized and accepted by Gatekeeper"
