---
type: patch
---

perf(cef): start the window and pane pools once the main window has loaded, not when its browser is created, so startup doesn't run three renderers at once
