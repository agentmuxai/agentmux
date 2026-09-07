---
type: patch
---

fix(srv): migrations run under a cross-process lock — two srv processes booting against the same data dir can no longer both apply the same migration (migration hardening Phase 3)
