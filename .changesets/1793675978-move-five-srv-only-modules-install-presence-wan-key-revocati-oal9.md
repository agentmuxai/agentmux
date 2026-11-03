---
type: patch
---

Move five srv-only modules (install presence, WAN key revocation, transcript request, remote op, work store) out of agentmux-common into srv, so a change to them no longer rebuilds the host, launcher and mcp
