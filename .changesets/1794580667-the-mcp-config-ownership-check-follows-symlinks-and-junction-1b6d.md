---
type: patch
---

The MCP config ownership check follows symlinks and junctions before it collapses .. in a working directory, so a project reached through link/.. is not read as AgentMux's own folder
