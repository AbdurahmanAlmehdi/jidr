# Button

Primary, secondary, tertiary, destructive and icon buttons.

**Consumer provides:** label (a verb), optional icon and shortcut, state.

- **One accent (primary) button per surface**, for the thing the surface is for. Everything else secondary or tertiary.
- Heights `jidr-control-height-sm|md|lg` (24/30/44). Primary `jidr-color-accent-fill` → hover `-hover` → pressed `-pressed`; all keep 4.5:1 with `jidr-color-text-on-accent`.
- Loading: verb in progress (“Starting…”), spinner replaces the icon, shortcut hidden. Error exists only as Retry after a failed action.
- Icon buttons always have an `aria-label` and a tooltip with the shortcut. Destructive fill is never the default button.
