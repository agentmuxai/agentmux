---
type: patch
---

Frontend: the store runs the browser sleep, wake and redock commands and counts terminals through a small view-actions registry, and the Connectors and Memory pane helpers move into the store, so store no longer imports a view for them (inverted imports 18 to 14)
