---
type: patch
---

A container agent is never made from the retired agent-claude image (it becomes the base image, never the developer one, and a container already made from it is recreated), a container whose image lacks agentmux-bashwrap or agentmux-mcp is refused at start with a plain message and stopped, and update_container changes the image the next start uses
