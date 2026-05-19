---
type: patch
---

feat(launch-modal): migrate form state to launch-flow-state slice (Stage 2b)

AgentLaunchModal's form fields (name, runtime, image, identity,
memory, continueOf) now route through the launch-flow-state
reducer. Adds the Solid-store wrapper + tests; the existing
accessor names (`name()`, `setName(v)`, …) remain as thin facades
so call sites are unchanged.

Stage 2c will migrate the resources (identities/memories/bindings)
+ wire backend push events for cross-tab reactivity.
