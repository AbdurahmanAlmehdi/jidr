# JobRow

A job in progress: download, root patch or verification, with cancel.

**Consumer provides:** job kind, title, progress 0–1, bytes done/total or step index, state, error message.

- The bar follows real bytes or real steps, never time. Unknown length: spinner and step name, no bar.
- Root setup lists its five steps (Download Magisk › Patch › Reboot › Install › Verify) with the current one in `jidr-color-info`.
- Cancel (⌘.) is an icon button; finished jobs swap it for Retry (failed) or nothing, and leave the list after 10 s unless they failed.
- Numbers in `type-metric`.
