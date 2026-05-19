---
type: patch
---

feat(launch-modal): route auth state through the slice (Stage 2d.2)

AuthFlowController accepts optional `externalGetState` +
`externalDispatch` hooks. AgentLaunchModal wires them so the
controller reads + writes auth state via the launch-flow-state
slice. `flow.state.auth` is now the single source of truth;
internal signal stays as a fallback for tests + standalone use.

§6.7 satisfied: all Launch modal state is owned by the slice.
