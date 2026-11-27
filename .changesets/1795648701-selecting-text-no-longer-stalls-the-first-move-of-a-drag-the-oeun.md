---
type: patch
---

Selecting text no longer stalls the first move of a drag: the selection guards stop restyling the whole page (the field keeps the pointer by capture, the rest of a pane drag is switched off a few roots at a time), and each drag logs its lag (#48733)
