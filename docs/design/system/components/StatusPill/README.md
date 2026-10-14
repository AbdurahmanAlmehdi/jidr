# StatusPill

A device's run state as a pill: icon plus label on the state's container colour.

**Consumer provides:** status, plus boot % (Booting), uptime (Running) or holder (Busy: CLI or agent).

- Pairs are shared: Starting, Booting, Stopping use `jidr-status-transition-*` and the spinner; Stopped and Paused use `jidr-status-neutral-*` (icons differ); Running `positive`; Crashed `negative`; Busy `warning`.
- Always read colours through `jidr-status-device-<state>-container` / `-on-container`, never the shared pair directly, so a state can be re-mapped in one place.
- The spinner stops under Reduce Motion; the label and percentage carry the state.
