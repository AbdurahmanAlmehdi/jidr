# DeviceRow

A device in the launcher list: silhouette, name with profile · Android · image, status pill, root badge, disk, and inline actions.

**Consumer provides:** the device record from the daemon (name, profile, Android version + API, image tag, status, root mode + health + Magisk version, disk GB), selection, density.

- Default density: two lines, `jidr-density-default-row-height` (52). Compact: one line, `jidr-density-compact-row-height` (30); Magisk version moves to the tooltip.
- Inline actions (Start/Stop, Open, More) appear on hover, focus and selection. Shortcuts: ↵ open, ⌘R start, ⌘. stop, ⌫ delete.
- Status and root are always visible on every row; never hide them to save space.
- Error = crashed: row tinted `jidr-status-negative-container`. Disabled = image deleted.
- Disk uses `type-metric` and stays LTR in Arabic.
