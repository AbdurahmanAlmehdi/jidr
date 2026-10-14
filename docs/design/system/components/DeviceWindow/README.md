# DeviceWindow

The device tab: the live Android screen in our bezel, a vertical toolbar and the widget panel host.

**Consumer provides:** the screen frames, device class, toolbar actions, the open widget.

- The Android screen is the hero. Nothing overlaps it; chrome around it is quiet.
- Toolbar `jidr-window-device-toolbar-width` (48) of 32 px icon buttons; every button has a tooltip with its shortcut.
- Widget panel `jidr-window-device-widget-panel-width` (320), ⌘J. A segmented control switches the four most-used widgets; the rest sit behind ⋯ and in ⌘K.
- Framing: Fit by default (largest size that fits, aspect kept); Pixel-perfect 1:1 (⌘0) scrolls the stage instead of scaling. Letterbox is `jidr-color-surface-sunken`, never decorated.
- In RTL the toolbar and panel swap sides and Back flips; the Android screen itself does not mirror.
