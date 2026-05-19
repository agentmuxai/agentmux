---
type: patch
---

feat(launch-modal): migrate submit + error state to launch-flow-state slice (Stage 2c.1)

AgentLaunchModal's `submitting` / `error` signals now live in the
reducer. handleSubmit dispatches `SubmitClicked` / `SubmitFailed`
directly instead of the legacy paired setters. The failure case now
sets in-flight=false and error=msg in one atomic dispatch.

Stage 2c.2 (resources) + 2c.3 (bindings + push events) land in
follow-up PRs.
