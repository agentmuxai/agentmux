---
type: patch
---

Browser panes keep up with the chrome around them on a shift-resize: the page sends their rects to the host over a CEF process message instead of HTTP, with HTTP kept as the fallback
