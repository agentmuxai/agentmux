---
type: patch
---

A terminal flood no longer lets the write-ahead log grow to several times the stored output: the writer now empties it by the log it has written, not the output.
