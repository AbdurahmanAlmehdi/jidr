#!/usr/bin/env bash
# End-to-end root cycle on real emulators: none → adb → magisk → none,
# verifying each step and that the shared SDK ramdisk.img is untouched at the
# end (Phase 9d acceptance).
#
# Env: JIDR_TEST_SDK    (SDK holding the images, read-only; default Studio's)
#      JIDR_TEST_IMAGES (space-separated sdkmanager packages; default one
#                        google_apis image for the host ABI — CI passes
#                        API 30/33/35 x86_64 on Linux KVM)
#      JIDR_TEST_MAGISK (optional local Magisk APK; default: download the pin)
set -euo pipefail
cd "$(dirname "$0")/.."

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/agent-c}"
cargo build -q -p jidr
JIDR="$PWD/$CARGO_TARGET_DIR/debug/jidr"

case "$(uname -m)" in arm64|aarch64) ABI=arm64-v8a ;; *) ABI=x86_64 ;; esac
SDK="${JIDR_TEST_SDK:-${ANDROID_HOME:-$HOME/Library/Android/sdk}}"
export ANDROID_HOME="$SDK"
IMAGES="${JIDR_TEST_IMAGES:-system-images;android-31;google_apis;$ABI}"

JIDR_HOME="$(mktemp -d /tmp/jidr-e2e-root.XXXXXX)"
export JIDR_HOME
cleanup() {
  mkdir -p "$JIDR_HOME/cleanup-home"
  HOME="$JIDR_HOME/cleanup-home" "$JIDR" --quiet uninstall --purge --yes >/dev/null 2>&1 || true
  "$JIDR" daemon stop >/dev/null 2>&1 || true
  rm -rf "$JIDR_HOME"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# Runs `jidr --json …` and checks ok == true; sets LAST to the last line.
step() {
  local name="$1"; shift
  local out code=0
  out="$("$JIDR" --json "$@")" || code=$?
  if [ "$code" -ne 0 ]; then
    echo "e2e-root: FAIL $name (exit $code): $out" >&2
    exit 1
  fi
  python3 -I -c 'import json,sys; v=json.loads(sys.stdin.read().strip().splitlines()[-1]); assert v["jidr_json"]==1 and v["ok"] is True, v' <<<"$out" \
    || { echo "e2e-root: FAIL $name: bad envelope: $out" >&2; exit 1; }
  echo "e2e-root: ok   $name"
  LAST="$out"
}

field() {  # field <python expression on r = result>
  python3 -I -c "import json,sys; r=json.loads(sys.stdin.read().strip().splitlines()[-1])['result']; print($1)" <<<"$LAST"
}

# SHA-256 of an image's pristine ramdisk.img, from its sdkmanager package.
pristine_hash() {
  local img="$1"
  local rel
  rel="$(printf '%s' "$img" | tr ';' '/')/ramdisk.img"
  shasum -a 256 "$SDK/$rel" | awk '{print $1}'
}

cycle() {
  local img="$1" dev="e2e-root" before after uid
  before="$(pristine_hash "$img")"
  echo "e2e-root: image $img (pristine ramdisk $before)"

  step "create" device create "$dev" --image "$img" --profile small-phone-5 --root-mode none
  step "start"  device start "$dev" --timeout 300

  # none: the shell is not root.
  step "verify-none" root verify "$dev"
  uid="$(field 'r["uid"]')"
  [ "$uid" != 0 ] || { echo "e2e-root: FAIL none is root ($uid)" >&2; exit 1; }

  # none → adb, live.
  step "set-adb" root set "$dev" adb
  uid="$(field 'r["verification"]["uid"]')"
  [ "$uid" = 0 ] || { echo "e2e-root: FAIL adb uid=$uid" >&2; exit 1; }

  # adb → magisk: the full setup, then `su -c id` must be uid 0.
  if [ -n "${JIDR_TEST_MAGISK:-}" ]; then
    step "set-magisk" root set "$dev" magisk --magisk "$JIDR_TEST_MAGISK"
  else
    step "set-magisk" root set "$dev" magisk
  fi
  uid="$(field 'r["verification"]["uid"]')"
  [ "$uid" = 0 ] || { echo "e2e-root: FAIL magisk su uid=$uid" >&2; exit 1; }
  [ "$(field 'r["verification"]["command"]')" = "su -c id" ] \
    || { echo "e2e-root: FAIL magisk probe: $LAST" >&2; exit 1; }
  echo "e2e-root:      magisk $(field 'r["magisk_version"]')"

  # magisk → none: revert.
  step "set-none" root set "$dev" none
  uid="$(field 'r["verification"]["uid"]')"
  [ "$uid" != 0 ] || { echo "e2e-root: FAIL revert is root ($uid)" >&2; exit 1; }

  step "stop"   device stop "$dev"
  step "delete" device delete "$dev" --yes

  after="$(pristine_hash "$img")"
  [ "$before" = "$after" ] || {
    echo "e2e-root: FAIL SDK ramdisk.img changed: $before → $after" >&2
    exit 1
  }
  echo "e2e-root:      SDK ramdisk.img unchanged"
}

start=$(date +%s)
for img in $IMAGES; do
  cycle "$img"
done
echo "e2e-root: passed in $(( $(date +%s) - start )) s"
