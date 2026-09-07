---
type: patch
---

fix(launcher,cef): migration-failure reason can no longer lose a select! race to the closed ESTART channel — biased poll order plus a post-loop buffered-reason check, so a failed migration always surfaces as MigrationFailed instead of degrading to the generic channel-closed error
