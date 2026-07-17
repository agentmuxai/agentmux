# SPEC: Pillar 1 Step 6 — collapse launcher saga durability to an in-memory registry

**Date:** 2026-07-16
**Status:** Implemented (this PR)
**Tracking:** Pillar 1 Step 6 (`docs/architecture/DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md` §3; `docs/status/STATUS_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_07_16.md` §1)
**Supersedes (mechanism, not history):** `SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` (LSD-1..4) — that spec's durable log, recovery walker, retention vacuum, and `--diag sagas` offline reader are deleted by this change; its live coordinator semantics are retained unchanged.

---

## 1. Why now instead of the August bake gate

The bake period existed to accumulate evidence that crash-reproject is trustworthy before deleting the durable-saga fallback. Re-examination (2026-07-16, with the user) showed the calendar gate validates nothing about this layer, because **the durable layer has no crash-time behavior to validate**:

- `recovery.rs` (deleted), in its own words: *"Critical: we DO NOT auto-replay or auto-compensate launcher sagas."* On restart it marked interrupted rows `failed_compensation` — a diagnostic tombstone for `--diag sagas` — and did nothing else. Confirmed empirically on this machine's launcher logs: every "recovery" firing was a mark-and-continue; zero behavioral compensation ever ran.
- Both concrete sagas are **narrators** of cleanup the host performs organically:
  - `window_cleanup_cascade` (ReapPanes → DrainPoolIfLast): the host's `on_before_close`/close-path does the actual reaping and drain-decision; the saga's `saga_id`-stamped commands are echo/no-op narration (`saga_dispatch.rs` `reap_panes` explicitly "relies on the organic report").
  - `pool_respawn_on_promote` (SpawnPoolWindow): warm-pool refill — a lazily-recoverable optimization, not session state.
- Neither saga has any compensation ("Compensation: none" in both files' docs). A crash mid-saga loses nothing srv doesn't already own; crash-reproject (Steps 1–5, E2E-tested in CI) rebuilds the session, and the teardown backstop (#2187) reaps a wedged host.

What the bake DOES protect — reproject itself — is guarded continuously by the Step 5 E2E test, not by the calendar.

## 2. What changed

| Piece | Before | After |
|---|---|---|
| `saga/log/mod.rs` | SQLite (`launcher-sagas.db`, WAL, schema migrations) | In-memory `BTreeMap` registry behind the **same method surface**; terminal-saga retention bounded at 128 entries (replaces the vacuum) |
| `saga/log/schema.rs` | DDL + version stamping | **Deleted** |
| `saga/recovery.rs` | Startup walker marking interrupted rows `failed_compensation` | **Deleted** (a fresh process has an empty registry by construction) |
| `saga/log/tests.rs` | SQLite contract tests | Ported behavioral contract (lifecycle states, unresolved semantics, failure-reason append, snapshot ordering, duplicate-id surfacing) + new retention-cap test; SQLite-specific tests (schema idempotence, FK pragma, vacuum) deleted with their subject |
| Supervisors (`windows.rs`, `unix.rs`) | Open db (FATAL on failure) → recovery walk → vacuum → coordinator | `LauncherSagaLog::new()` + best-effort deletion of legacy `launcher-sagas.db{,-wal,-shm}` files → coordinator. The "saga recovery" splash stage is gone (it no longer exists as work) |
| `--diag sagas` | Offline read-only SQLite inspector | Prints an explanation + points at the launcher log's `[saga]` lines (the live narration, unchanged) |
| `config.rs` | `[saga.launcher] retention_days` config | **Deleted** (whole module — retention is the in-memory cap) |
| `data_dir.rs` | `launcher_saga_log_path{,_read_only}` + legacy-file migration | **Deleted** |
| `Cargo.toml` | `rusqlite` (bundled SQLite) | **Removed** — the launcher no longer links SQLite at all |

**Unchanged:** the saga *coordinator* (`saga/mod.rs`) — trigger matching, `saga_id` injection/correlation (CPD-3..5), timeouts, `cancel_all_in_flight` on clean shutdown, the `[saga]` launcher-log narration, and the host-side `saga_dispatch` echo/idempotency layer. srv's saga layer (which performs real reducer-level compensation) is entirely out of scope.

## 3. What is lost, explicitly

- **Offline post-crash saga forensics** (`--diag sagas` reading rows from a dead launcher's db). Per the deleted recovery.rs, this was the durable layer's only real product. The live `[saga]` log lines (which we used as the verification signal for #2186's PoolDrained fix the same day) carry the same narrative and survive in the launcher log files.
- Nothing else. `saga_id` correlation, live diagnostics via `snapshot_recent`/`unresolved_sagas` (now in-process), and every coordinator behavior are preserved.

## 4. Follow-ups (not this PR)

- `orphan_reconcile.rs` (836 lines, host-side) — the other artifact of the flush-vs-crash incoherence. Pillar 2's sanitize-then-decide already demoted it to sanitizer/executor; further shrink needs its own analysis.
- `SagaCoordinator` API simplification (e.g. `with_log` can become infallible now that `max_saga_id` cannot fail) — cosmetic, deferred to avoid churn in this PR.

## 5. Verification

- `cargo test -p agentmux-launcher` — 206/206 (ported contract tests + new retention-cap test).
- Live (Windows dev): launcher startup logs `[saga] removed legacy durable saga log file …` once per stale file; window close still produces the full `[saga] … window_cleanup_cascade … Done — emitting SagaCompleted` narration; no `launcher-sagas.db` is recreated; `--diag sagas` prints the Step-6 notice.
