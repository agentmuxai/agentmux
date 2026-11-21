---
type: patch
---

The agent runtime store keeps a signal for each agent, so an event for one agent re-runs only what reads that agent, and an event that changes nothing re-runs nothing
