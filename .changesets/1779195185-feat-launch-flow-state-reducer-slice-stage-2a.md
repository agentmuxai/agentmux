---
type: patch
---

feat(launch-flow-state): additive reducer slice + tests (Stage 2a)

Adds `frontend/app/store/launch-flow-state/` — types + pure reducer
+ selectors + 37 unit tests covering the form/identity/memory/
bindings/submit cross-product. Modeled on browser-pane-state.

Purely additive — no view migration yet. AgentLaunchModal still uses
its existing local signals; Stage 2b migrates it. Spec:
docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md
