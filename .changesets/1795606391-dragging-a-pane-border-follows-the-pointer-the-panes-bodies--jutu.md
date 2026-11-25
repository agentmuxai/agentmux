---
type: patch
---

Dragging a pane border follows the pointer: the panes' bodies catch up one at a time in spare frames and when the pointer pauses, the browser pane's position poll waits, and each drag logs how far the border lagged (#48711)
