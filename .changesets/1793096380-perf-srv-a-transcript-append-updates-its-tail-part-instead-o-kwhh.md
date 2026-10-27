---
type: patch
---

perf(srv): a transcript append updates its tail part instead of replacing it, so SQLite no longer spills a statement journal to a temp file for every line (about a third of the bytes srv wrote in the memory lab)
