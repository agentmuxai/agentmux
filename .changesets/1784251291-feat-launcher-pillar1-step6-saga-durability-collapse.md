---
type: patch
---

feat(launcher): Pillar 1 Step 6 — collapse saga durability to an in-memory registry

Deletes the launcher's durable SQLite saga layer (launcher-sagas.db, schema migrations, the startup recovery walker, the retention vacuum, the [saga.launcher] retention config, the rusqlite dependency, and --diag sagas' offline reader) and replaces it with an in-memory registry behind the same coordinator API. The durable layer's crash-time behavior was, by its own design, a no-op — recovery never replayed or compensated anything, it only wrote failed_compensation tombstones for operator review — and with srv authoritative + crash-reproject (Steps 1–5) both concrete sagas are pure narrators of cleanup the host performs organically. Live coordinator semantics (triggers, saga_id correlation, timeouts, [saga] log narration, clean-shutdown cancel) are unchanged. Legacy on-disk saga db files are deleted at first startup. Terminal-saga retention is a bounded in-memory cap (128). Completes the disposable-host program's final step; gate lifted early with evidence (see SPEC_PILLAR1_STEP6_SAGA_COLLAPSE_2026_07_16.md §1).
