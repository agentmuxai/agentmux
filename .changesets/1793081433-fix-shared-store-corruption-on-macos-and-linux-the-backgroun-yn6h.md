---
type: patch
---

Fix shared-store corruption on macOS and Linux: the background checkpointer no longer opens and closes the WAL index file, which released SQLite's locks mid-transaction
