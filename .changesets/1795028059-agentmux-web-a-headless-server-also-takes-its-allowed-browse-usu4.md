---
type: patch
---

AgentMux Web: a headless server takes its one allowed browser origin from the AGENTMUX_ALLOWED_ORIGIN environment variable (https only, and it must agree with --allowed-origin if both are given), so a launcher can set it per run
