---
type: patch
---

The backup taken before migrations no longer misses recent changes: it copies each database through SQLite instead of copying the file, and also covers identity-store.db
