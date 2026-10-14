---
type: patch
---

fix(srv): a signal reaching srv's launcher watcher no longer quits the whole app; the Linux poll() and macOS kevent() waits retry on EINTR
