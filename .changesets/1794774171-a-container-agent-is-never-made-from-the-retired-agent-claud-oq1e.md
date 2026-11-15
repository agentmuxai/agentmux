---
type: patch
---

A container agent is never made from the retired agent-claude image, a container whose image lacks agentmux-bashwrap or agentmux-mcp is refused at start with a plain message, and update_container changes the image the next start uses
