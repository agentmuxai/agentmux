---
type: patch
---

feat(srv): agentmux-srv migrate --verify — a doctor pass that asks every APPLIED migration for its post-condition (row counts, marker files) and exits 3 on any mismatch; 0007 and 0002 implement real checks (migration hardening Phase 1b)
