#!/usr/bin/env bash
# End-to-end CLI run on a real emulator, non-interactive, JSON everywhere:
# create → start (wait for boot) → install → stop → delete → daemon stop.
#
# Env: JIDR_TEST_SDK (SDK holding the image, read-only; default Studio's),
#      JIDR_TEST_IMAGE (sdkmanager package; default API 31 google_apis, host ABI),
#      JIDR_TEST_APK (APK to install; default a sample app from the SDK's images).
set -euo pipefail
cd "$(dirname "$0")/.."

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/agent-a}"
cargo build -q -p jidr
JIDR="$PWD/$CARGO_TARGET_DIR/debug/jidr"

case "$(uname -m)" in arm64|aarch64) ABI=arm64-v8a ;; *) ABI=x86_64 ;; esac
SDK="${JIDR_TEST_SDK:-${ANDROID_HOME:-$HOME/Library/Android/sdk}}"
export ANDROID_HOME="$SDK"
IMAGE="${JIDR_TEST_IMAGE:-system-images;android-31;google_apis;$ABI}"
APK="${JIDR_TEST_APK:-$(find "$SDK/system-images" -path '*/data/app/*' -name '*.apk' 2>/dev/null | head -1 || true)}"
if [ -z "$APK" ] || [ ! -f "$APK" ]; then
  echo "e2e-cli: no APK to install; set JIDR_TEST_APK" >&2
  exit 1
fi

# A short, private Jidr home (Unix socket paths are limited to ~104 bytes).
JIDR_HOME="$(mktemp -d /tmp/jidr-e2e.XXXXXX)"
export JIDR_HOME
cleanup() {
  "$JIDR" daemon stop >/dev/null 2>&1 || true
  rm -rf "$JIDR_HOME"
}
trap cleanup EXIT

# Runs `jidr --json …`, checks the exit code and that ok == true, prints the result.
step() {
  local name="$1"; shift
  local out code=0
  out="$("$JIDR" --json "$@")" || code=$?
  if [ "$code" -ne 0 ]; then
    echo "e2e-cli: FAIL $name (exit $code): $out" >&2
    exit 1
  fi
  python3 -I -c 'import json,sys; v=json.loads(sys.stdin.read().strip().splitlines()[-1]); assert v["jidr_json"]==1 and v["ok"] is True, v' <<<"$out" \
    || { echo "e2e-cli: FAIL $name: bad envelope: $out" >&2; exit 1; }
  echo "e2e-cli: ok   $name"
  LAST="$out"
}

field() {  # field <python expression on r = result>
  python3 -I -c "import json,sys; r=json.loads(sys.stdin.read().strip().splitlines()[-1])['result']; print($1)" <<<"$LAST"
}

start=$(date +%s)
step "create"   device create e2e --image "$IMAGE" --profile small-phone-5
step "start"    device start e2e --timeout 300
[ "$(field 'r[0]["status"]["state"]')" = running ] || { echo "e2e-cli: FAIL not running: $LAST" >&2; exit 1; }
echo "e2e-cli:      booted on $(field 'r[0]["runtime"]["adb_serial"]') after $(( $(date +%s) - start )) s"
step "install"  install e2e "$APK"
step "logcat"   logcat e2e -n 20
step "stop"     device stop e2e
step "delete"   device delete e2e --yes
step "list"     device list
[ "$(field 'len(r)')" = 0 ] || { echo "e2e-cli: FAIL device left behind: $LAST" >&2; exit 1; }

# Uninstall --purge (D31) with HOME pointed at a scratch dir, so the real MCP
# configs are never touched; a fake Claude Code config proves the cleanup.
step "create2"  device create e2e2 --image "$IMAGE" --profile small-phone-5
step "start2"   device start e2e2 --timeout 300
FAKE_HOME="$JIDR_HOME/home"
mkdir -p "$FAKE_HOME"
printf '{"mcpServers":{"jidr":{"command":"jidr","args":["mcp"]},"other":{"command":"x"}}}' > "$FAKE_HOME/.claude.json"
LAST="$(HOME="$FAKE_HOME" "$JIDR" --json uninstall --purge --yes)" || { echo "e2e-cli: FAIL uninstall: $LAST" >&2; exit 1; }
echo "e2e-cli: ok   uninstall --purge"
[ "$(field 'r["purged"]')" = True ] || { echo "e2e-cli: FAIL not purged: $LAST" >&2; exit 1; }
for d in data cache logs; do
  [ ! -e "$JIDR_HOME/$d" ] || { echo "e2e-cli: FAIL $JIDR_HOME/$d remains" >&2; exit 1; }
done
python3 -I -c 'import json,sys; c=json.load(open(sys.argv[1])); assert "jidr" not in c["mcpServers"] and "other" in c["mcpServers"], c' "$FAKE_HOME/.claude.json" \
  || { echo "e2e-cli: FAIL MCP entry not removed" >&2; exit 1; }
[ ! -S "$JIDR_HOME/data/daemon.sock" ] || { echo "e2e-cli: FAIL daemon socket remains" >&2; exit 1; }
# No emulator may still be running a Jidr AVD.
python3 -I "$(dirname "$0")/lib/no_jidr_emulators.py" "$JIDR_HOME" \
  || { echo "e2e-cli: FAIL a Jidr emulator is still running" >&2; exit 1; }
echo "e2e-cli: passed in $(( $(date +%s) - start )) s"
