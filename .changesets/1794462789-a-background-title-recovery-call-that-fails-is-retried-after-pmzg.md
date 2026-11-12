---
type: patch
---

A background title-recovery call that fails is retried after a growing wait (up to half an hour) instead of every 20 seconds, its failure reason is logged, and an agent on a model server of its own is no longer tried
