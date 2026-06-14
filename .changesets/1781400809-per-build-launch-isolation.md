---
type: patch
---

fix(launcher): isolate each local build as its own AgentMux instance — bake a per-build BUILD_ID into the data-dir channel (data dir + cef-cache + pipe now all per-build) and make a nested portable ignore the leaked ambient AGENTMUX_CHANNEL. Completes #1315 (which fixed only the pipe; cef-cache stayed per-branch). Safe now that agents + auth are global (#1387-#1393).
