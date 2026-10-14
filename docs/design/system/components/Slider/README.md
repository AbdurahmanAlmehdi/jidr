# Slider

Slider with a linked value field, used for battery level.

**Consumer provides:** value, min/max, unit, whether the device is running.

- Dragging updates the field; typing moves the thumb. Big readout in `type-metric-lg`.
- Out-of-range input shows an inline error; the value is not sent until valid.
- Disabled while the device is stopped, with a hint saying why.
