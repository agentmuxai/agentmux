---
type: patch
---

fix(srv): a failed data migration is now fatal — srv emits AGENTMUXSRV-MIGRATION-FAILED and exits 1 before ESTART instead of booting against a half-migrated store; launcher and CEF host surface the reason immediately (migration hardening Phase 1a)
