---
type: patch
---

feat(launch-modal): migrate identities + memories resources to slice (Stage 2c.2)

AgentLaunchModal's `identities` and `memories` createResource calls
are replaced with async wrappers that dispatch
IdentitiesLoading/Loaded/Failed (and the Memory equivalents) into
the reducer. `realIdentities` / `realMemories` selectors replace
the inline `is_blank` filter memos.

Stage 2c.3 (bindings migration + identitybundlebindings:changed
subscription) lands next.
