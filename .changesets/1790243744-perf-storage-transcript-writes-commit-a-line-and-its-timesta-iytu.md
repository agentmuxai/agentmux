---
type: patch
---

perf(storage): transcript writes commit a line and its timestamp together, use SQLite synchronous=NORMAL, and checkpoint in the background, so a live agent line is saved in ~1 ms instead of ~8
