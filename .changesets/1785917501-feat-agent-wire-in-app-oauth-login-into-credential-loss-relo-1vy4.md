---
type: minor
---

feat(agent): wire in-app OAuth login into credential-loss relogin, retiring AuthUrlBox

Mid-session credential-loss relogin now renders the shared `InAppLoginPanel` (the same UI the Armory/Stash surfaces use) inside `AgentAuthPanel`'s bottom-docked slot — next to the composer, same as `AgentQuestionPanel`/`AgentDecisionPanel` — instead of the old hand-rolled `AuthUrlBox`, which is fully retired along with its dead CSS.
