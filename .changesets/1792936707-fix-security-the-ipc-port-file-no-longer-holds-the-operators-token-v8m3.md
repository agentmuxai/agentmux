---
type: patch
---

fix(security): the host's ipc-port file no longer holds the token that proves a request comes from the operator's window; it holds a forwarding token that opens only a new window, raising a window and quitting
