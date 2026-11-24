---
type: patch
---

The agent pane's line count no longer holds a runtime worker while the stores are busy: it is answered on the blocking pool.
