---
type: patch
---

Closing an agent in one AgentMux instance now frees it for another instance on the same computer. The closed agent could stay in the cloud subscription and keep its cloud lease forever, so reopening it elsewhere said it was running elsewhere; such leftovers are now dropped within 20 seconds. That refusal also now says "on this computer" instead of "on another computer" when the other instance is on the same machine.
