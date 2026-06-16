---
type: patch
---

fix(agent-session): read global snapshot first so cross-channel opens never get a stale per-channel sourceBlockId
