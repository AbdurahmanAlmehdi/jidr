# DeviceCard

A device in the grid view: a stage with our neutral silhouette and status pill, then name, profile, root badge, Android and disk, and one or two actions.

**Consumer provides:** the same device record as DeviceRow.

- At most one accent button per card: Start when stopped. Running devices get a secondary Open; Busy devices get Watch.
- Selected: `jidr-color-accent-fill` outline. Card on `jidr-color-surface-raised`, stage on `jidr-color-surface-sunken`, radius `jidr-radius-lg`.
- Silhouettes are plain rounded rectangles sized by device class. No brand shapes.
