---
type: patch
---

fix(security): the host's IPC token is no longer put in a window's address (it could reach browser history and caches); the host hands it to the page as the page starts loading
