---
type: patch
---

writeagentconfig refuses a connection that is neither the AgentMux window nor an agent writing its own workspace, so an agent can't rewrite another agent's config by using the instance key without registering
