---
type: patch
---

The agent picker and the Toolchain pane now agree on whether a harness's CLI is installed. Harnesses AgentMux doesn't install (Grok, Goose, Kimi and others) show "Click to install" when their CLI isn't found, and the Toolchain no longer reports a CLI you installed yourself as an npm harness's own copy
