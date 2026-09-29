---
type: patch
---

perf(layout): each pane no longer re-checks its drag handle every 100 ms for as long as it is open; it rebinds only when its header actually changes
