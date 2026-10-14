# CreateDeviceSheet

New-device sheet. **A single form, not a stepper.**

Every field has a default, so the common path is name → ⌘↵. A stepper would put four clicks between the user and a device and bury root mode on step three. The form keeps root mode visible, folds rare hardware settings into Advanced, and validates inline.

**Consumer provides:** hardware profiles, installed and downloadable images (with sizes), default root mode.

- Duplicate names show an inline error and disable Create.
- First-run uses the same sheet with the downloaded image preselected.
