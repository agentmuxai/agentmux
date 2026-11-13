---
type: patch
---

A terminal that prints without end no longer lets the store's write-ahead log grow on a slow disk: the writer's own checkpoint waits for the background one instead of being skipped
