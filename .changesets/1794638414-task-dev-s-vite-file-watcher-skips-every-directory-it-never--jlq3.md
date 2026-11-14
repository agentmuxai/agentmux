---
type: patch
---

task dev's Vite file watcher skips every directory it never serves (crates, docs, any target-<name> build dir) instead of holding a handle per file in them
