---
type: patch
---

A history read whose index is unavailable no longer loads a large transcript whole under the store lock (it returns an error the pane retries), and a read served from an index an append had outrun is logged
