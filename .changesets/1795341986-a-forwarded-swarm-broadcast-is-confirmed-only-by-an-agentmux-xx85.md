---
type: patch
---

A forwarded Swarm broadcast is confirmed only by an AgentMux instance, not by a process inside an agent's process tree; if the instance cannot be identified the forward arrives as an ordinary message and the audit says so
