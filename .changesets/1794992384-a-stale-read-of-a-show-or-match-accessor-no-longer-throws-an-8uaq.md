---
type: patch
---

A stale read of a <Show> or <Match> accessor no longer throws and drops the window's queued updates; it answers with the last value and logs the reader's stack
