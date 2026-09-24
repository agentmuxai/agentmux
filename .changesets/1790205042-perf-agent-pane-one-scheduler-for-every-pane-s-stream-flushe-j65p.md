---
type: patch
---

perf(agent-pane): one scheduler for every pane's stream flushes — while the user is typing, at most one pane flushes per frame (oldest first, 100 ms starvation guard), so keystrokes are handled between panes' updates
