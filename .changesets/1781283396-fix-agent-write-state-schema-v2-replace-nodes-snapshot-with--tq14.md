---
type: patch
---

fix(agent): write-state schema v2 — replace nodes[] snapshot with a lightweight overlay + NDJSON restore to eliminate the renderer OOM crash. v2 restore is scoped to same-block reopen (the OOM-critical path); cross-block "structural continuation" falls back to NDJSON replay until a unified per-agent log lands (follow-up).
