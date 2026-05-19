---
type: patch
---

feat(launch-flow-state): wire recordDispatch audit ring (closes §6.8)

createLaunchFlowStore now appends every dispatch to the global
audit ring (frontend/app/store/command-source.ts). Each entry tags:
`slice: "launch-flow-state"`, `key: null`, the command + emitted
events + source ("user" default, "system" override). The diag panel
gets transition history for free.

Closes the final unchecked acceptance criterion of
docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md.
