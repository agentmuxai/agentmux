---
type: patch
---

Build folders are cleaned on a host that runs a local package: any srv started by its launcher on the real home may run the cleanup, one at a time per host, and AGENTMUX_BUILD_CACHE_CLEAN=0/1 overrides it
