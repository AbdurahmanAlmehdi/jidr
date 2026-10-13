#!/usr/bin/env bash
# Proves Jidr never writes to Android Studio's SDK or AVDs (D3, D31).
#
#   scripts/e2e-studio-untouched.sh [command...]
#
# Fingerprints every file (path, size, mtime) under the Studio SDK and
# ~/.android/avd, runs the command (default: the jidr-sdk emulator tests),
# and fails if anything changed. AVDs that are running right now are
# skipped, since the emulator legitimately writes to them.
set -euo pipefail

STUDIO_SDK="${JIDR_TEST_SDK:-${ANDROID_HOME:-$HOME/Library/Android/sdk}}"
STUDIO_AVD="${ANDROID_AVD_HOME:-$HOME/.android/avd}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/agent-a}"

if [ "$#" -eq 0 ]; then
  set -- cargo nextest run -p jidr-sdk --features emulator --no-tests=pass
fi

if [ ! -d "$STUDIO_SDK" ]; then
  echo "e2e-studio-untouched: no Studio SDK at $STUDIO_SDK; nothing to protect"
  exec "$@"
fi

# AVD dirs held by a live emulator (hardware-qemu.ini.lock with a live pid).
running_avds() {
  for lock in "$STUDIO_AVD"/*.avd/hardware-qemu.ini.lock; do
    [ -e "$lock" ] || continue
    pid=$(tr -dc '0-9' < "$lock" || true)
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      dirname "$lock"
    fi
  done
}

fingerprint() {
  local excludes=()
  while IFS= read -r dir; do
    excludes+=(-path "$dir" -prune -o)
  done < <(running_avds)
  {
    find "$STUDIO_SDK" -type f -print0
    if [ -d "$STUDIO_AVD" ]; then
      # (bash 3.2 on macOS: an empty array under `set -u` needs this form)
      find "$STUDIO_AVD" ${excludes[@]+"${excludes[@]}"} -type f -print0
    fi
  } | xargs -0 "${STAT[@]}" 2>/dev/null | LC_ALL=C sort
}

if [ "$(uname)" = Darwin ]; then STAT=(stat -f '%N %z %m'); else STAT=(stat -c '%n %s %Y'); fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "e2e-studio-untouched: fingerprinting $STUDIO_SDK and $STUDIO_AVD"
fingerprint > "$tmp/before"
echo "e2e-studio-untouched: $(wc -l < "$tmp/before" | tr -d ' ') files; running: $*"
"$@"
fingerprint > "$tmp/after"

if ! diff -u "$tmp/before" "$tmp/after" > "$tmp/diff"; then
  echo "e2e-studio-untouched: FAIL, Studio files changed:"
  head -50 "$tmp/diff"
  exit 1
fi
echo "e2e-studio-untouched: ok, $(wc -l < "$tmp/after" | tr -d ' ') files unchanged"
