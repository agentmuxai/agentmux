---
type: patch
---

fix(registry): cross-channel agent backfill now captures ALL existing agents — the one-shot definition migration was skipping whole DBs on a missing column (older schemas lack container_*), never scanned dev/ branches, and wrote an unconditional one-shot marker after that incomplete pass. Now: schema-resilient column handling (PRAGMA-introspect, default missing), scans channels AND dev, and a versioned marker that re-runs once so existing users recover their agents (Qooma, etc.) cross-channel+version. See ANALYSIS_CROSS_CHANNEL_AGENT_RETENTION_2026_06_13
