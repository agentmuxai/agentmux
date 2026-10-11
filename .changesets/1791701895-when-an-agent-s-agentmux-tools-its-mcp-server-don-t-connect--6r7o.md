---
type: minor
---

When an agent's AgentMux tools (its MCP server) don't connect, AgentMux now notices, reconnects them, and if that fails restarts the agent once without the shell prefix; the pane shows each step, the agent is told if it gives up, and `agentmux-mcp doctor` explains why from the agent's shell.
