---
type: patch
---

agentmux-bashwrap and agentmux-mcp are 1.2 MiB smaller each (about 30%): they only talk to the local AgentMux server over plain HTTP, so they no longer carry a TLS library
