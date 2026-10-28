---
type: patch
---

Every state-changing call on the websocket RPC channel is now recorded in the audit log as whoever made it (an agent, the AgentMux window proven by the host, or an unproven connection, never the operator by default), with the agent or pane it acted on and whether it was done or refused
