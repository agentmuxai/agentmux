---
type: patch
---

fix(srv): stop FsWatchPool's health sweep leaking a File+Semaphore handle pair per tick
