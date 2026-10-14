# SnapshotList

Saved device states with thumbnail, name, root-mode tag, size and time.

**Consumer provides:** snapshots (name, created at, size, root mode tag, PNG thumbnail).

- Thumbnail sits in our bezel. Quick boot is the automatic snapshot: tagged Auto, can't be renamed.
- Restoring a snapshot taken in a different root mode warns first.
- Restore is secondary; delete lives in ⋯.
