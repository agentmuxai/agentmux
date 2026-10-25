---
type: patch
---

fix(security): the two secrets the launcher hands srv and the host are wiped from their environment at startup, so /proc/<pid>/environ and ps no longer show them and no child process inherits them
