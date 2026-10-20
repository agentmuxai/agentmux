---
type: patch
---

The shared transcript store is checkpointed only when frames are waiting in its write-ahead log, its log file is capped at 64 MB, and one instance trims it every half hour; an idle store no longer runs four checkpoints a second
