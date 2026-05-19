---
type: patch
---

feat(launch-flow-state): fold AuthState into the slice (Stage 2d.1)

`LaunchFlowState.auth: AuthState` is now part of the slice. The
reducer delegates `{ type: "Auth", cmd }` commands to auth-state's
pure `update()` and wraps emitted events as
`{ type: "Auth", event }` on the outer ReducerResult.

Adds the §6.9 cross-product test suite — 8 tests asserting that
form-field commands (Name/Memory/Runtime/Image/Identities/
Bindings/Submit) never touch `state.auth`, and that auth commands
never touch `state.form`. Pins the original memory-change-resets-
auth bug as a pure-reducer regression.

View migration to read `flow.state.auth.kind` is Stage 2d.2.
