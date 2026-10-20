---
type: patch
---

Running a drone no longer fails with a foreign key error: drone runs are kept per version while drone definitions live in the shared store, and deleting a drone removes its runs
