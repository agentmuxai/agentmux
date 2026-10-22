---
type: patch
---

An agent now has one id everywhere: AgentMux reads it from the shared agent definition, a respawn takes it from there instead of an old saved launch environment, and deleting an agent removes its signing keys under every id it ever had.
