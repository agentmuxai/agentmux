---
type: minor
---

feat(launch-modal): lift auth controller out of conditional panel + remove blank Identity/Memory (Stage 1)

Fixes the "memory change → forgot login" repro by lifting the
AuthFlowController instance from PreLaunchAuthPanel up to
AgentLaunchModal so its lifetime spans the whole modal mount.
Conditionally re-rendering the Connect panel no longer destroys
in-flight auth state.

Also removes the "blank" Identity/Memory sentinel: both selections
are now required at submit. Spec:
docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md
