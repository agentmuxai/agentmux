---
type: patch
---

On Linux and macOS, each agent CLI now runs in its own process group: closing an agent ends the MCP servers it started, a signal sent to an agent's group can no longer stop AgentMux's background service, and if that service dies the launcher still ends the agents it left running.
