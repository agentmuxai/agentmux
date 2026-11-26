---
type: patch
---

A pane border drag's release no longer lays the whole page out again: the commit uses the container size the drag held, which took 130 to 320 ms in a window of many panes
