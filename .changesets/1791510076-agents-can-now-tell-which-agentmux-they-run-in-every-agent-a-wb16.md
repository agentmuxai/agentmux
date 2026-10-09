---
type: patch
---

Agents can now tell which AgentMux they run in: every agent and shell gets AGENTMUX_VERSION, AGENTMUX_BUILD (a local build's exact label) and AGENTMUX_INSTANCE_CHANNEL, and the Environment note tells agents to read them.
