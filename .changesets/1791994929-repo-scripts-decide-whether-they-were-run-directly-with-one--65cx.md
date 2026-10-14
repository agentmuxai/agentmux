---
type: patch
---

Repo scripts decide whether they were run directly with one shared check, so check-new-spec-status.mjs now actually runs on Windows (it used to exit 0 without checking anything).
