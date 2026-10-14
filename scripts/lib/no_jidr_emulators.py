"""Exit 1 if a live emulator is running an AVD under the given Jidr home.

usage: python3 -I scripts/lib/no_jidr_emulators.py <JIDR_HOME>
"""

import glob
import os
import sys

home = os.path.realpath(sys.argv[1])
dirs = [
    os.path.expanduser("~/Library/Caches/TemporaryItems/avd/running"),
    os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/nonexistent"), "avd/running"),
    f"/tmp/android-{os.environ.get('USER', '')}/avd/running",
]
for d in dirs:
    for f in glob.glob(os.path.join(d, "pid_*.ini")):
        try:
            vals = dict(line.strip().split("=", 1) for line in open(f) if "=" in line)
            pid = int(os.path.basename(f)[4:-4])
        except (OSError, ValueError):
            continue
        if not os.path.realpath(vals.get("avd.dir", "/")).startswith(home):
            continue
        try:
            os.kill(pid, 0)
        except OSError:
            continue
        print(f"still running: pid {pid} ({vals.get('avd.id')})", file=sys.stderr)
        sys.exit(1)
