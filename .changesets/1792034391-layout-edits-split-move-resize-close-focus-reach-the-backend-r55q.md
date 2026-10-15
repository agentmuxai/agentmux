---
type: patch
---

Layout edits (split, move, resize, close, focus) reach the backend as single edits that the backend computes and stores, instead of the whole pane tree; if the window's layout ever differs from the stored one, the window shows the stored one.
