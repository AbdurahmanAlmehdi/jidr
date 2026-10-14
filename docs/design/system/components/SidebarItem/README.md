# SidebarItem

One device in the sidebar: name and short root badge on line one, status glyph and label on line two.

**Consumer provides:** the device (name, status, root mode), selection and focus.

- Selected: `jidr-color-surface-selected`. Hover: `jidr-color-surface-control`. Keyboard focus: 2 px inset `jidr-color-focus-ring`.
- Status text colour follows the status meaning (`jidr-color-positive`, `-info`, `-warning`, `-negative`, else `jidr-color-text-secondary`) and always has its icon.
- Disabled means the system image is missing; the row stays visible so the user can re-download it.
