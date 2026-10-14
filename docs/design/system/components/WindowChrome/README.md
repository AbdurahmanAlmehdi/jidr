# WindowChrome

The launcher window: a title bar with traffic lights and the tab strip (Library plus one tab per open device), a sidebar of devices and jobs, a split content area and an optional inspector.

**Consumer provides:** the device list, open tabs, current selection, and whether the inspector is open.

- Title bar `jidr-window-titlebar-height` (44) on `jidr-color-surface-chrome`. Active tab sits on `jidr-color-surface-base` and joins the content.
- Tabs: Library first, then devices; ⌘1–9 switch, ⌘W closes. Each device tab shows its status glyph.
- Sidebar `jidr-window-sidebar-width` (248), inspector `jidr-window-inspector-width` (300); both resize and collapse (⌥⌘S, ⌥⌘I).
- Window minimum `jidr-window-launcher-min-width` × `jidr-window-launcher-min-height` (960 × 600).
- In RTL the whole window mirrors, traffic lights included, as macOS does for right-to-left system languages.
- When the window is inactive the traffic lights turn `jidr-color-border-strong`.
