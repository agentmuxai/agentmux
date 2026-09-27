---
type: patch
---

perf(history): the startup history index build waits 60 s and runs at background priority, so it no longer competes with opening your first agents; a search before then starts it early
