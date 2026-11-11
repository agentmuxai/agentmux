---
type: patch
---

Messaging an agent no longer routes to a stale second pane of it: a second pane's register is refused while the agent's own pane holds it, and a broadcast copy that only the relay queued is reported as queued_unconfirmed, not delivered
