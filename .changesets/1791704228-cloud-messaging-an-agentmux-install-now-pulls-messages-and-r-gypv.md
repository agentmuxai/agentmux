---
type: patch
---

Cloud messaging: an AgentMux install now pulls messages and renews agent leases for all its agents in one request instead of one per agent, so relay traffic no longer grows with every open agent. Older relays are still asked one agent at a time.
