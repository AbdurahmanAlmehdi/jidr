# RootBadge

Shows a device's root mode and health. Root state is unmistakable without colour: each state has its own label, icon and shape.

**Consumer provides:** root mode, root health, Magisk version, image tag (Play Store ⇒ Beta).

| State | Shape | Icon | Tokens |
|---|---|---|---|
| Not rooted | text only | minus | `jidr-color-text-secondary` |
| Shell root (adb) | filled neutral pill | terminal | `jidr-status-root-shell-*` |
| Full root | outlined pill | key-round | `jidr-status-root-full-*` |
| Setting up root… | outlined pill | loader-circle (spins) | `jidr-status-root-setting-up-*` |
| Rooted · Magisk 30.7 | **solid plate** (the only one) | badge-check | `jidr-status-root-verified-*` |
| Root unverified | outlined pill | circle-help | `jidr-status-root-unverified-*` |
| Root failed: reason | 2 px heavy outline | circle-x | `jidr-status-root-failed-*` |
| Beta | square uppercase tag after the badge, never alone | flask-conical | `jidr-status-root-beta-*` |

- No dashed outlines: Slint can't draw dashed borders.
- Hover shows the last verified time. Failed is always followed by its reason and offers Revert.
- Sizes: 24 px default, 20 px in rows and the sidebar.
