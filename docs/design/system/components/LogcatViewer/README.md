# LogcatViewer

Live logcat: level filter, search, follow toggle, monospace rows.

**Consumer provides:** the line stream (timestamp, level, tag, pid, message), filter, follow state.

- Columns: timestamp · pid · level · tag · message, in `type-code`. Timestamps keep logcat's threadtime format (`10-13 14:02:07.318`).
- Level colours `jidr-logcat-verbose` … `jidr-logcat-fatal` all hold 4.5:1 on `jidr-logcat-background`. The level letter is its own column and F rows get `jidr-logcat-fatal-row` and bold, so level never depends on hue.
- Level filter is a minimum. Follow pins to the newest line; scrolling up pauses it (⌘F to resume).
- Logcat stays LTR in the Arabic build.
