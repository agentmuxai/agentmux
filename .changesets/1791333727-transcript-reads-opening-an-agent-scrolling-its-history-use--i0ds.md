---
type: patch
---

Transcript reads (opening an agent, scrolling its history) use their own read-only database connections, so they no longer wait behind other agents' writes
