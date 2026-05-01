// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// LSD-1 — launcher saga log schema migration.
//
// See `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` §3.2 for
// the canonical schema. Mirrors srv's `run_saga_log_migrations` in
// `agentmux-srv/src/backend/storage/migrations.rs` but with two
// launcher-specific deltas:
//
//   1. A `target` column on the step table. Launcher sagas dispatch
//      to multiple peers (self / host / srv); srv sagas only ever
//      target the srv reducer so srv's schema can omit it.
//   2. A `failed_compensation` saga state. Launcher sagas don't
//      auto-compensate (LSD spec §3.5); recovery marks unresolved
//      sagas as `failed_compensation` for operator review. Srv has
//      a separate `compensated` terminal state instead.
//
// Schema lifecycle policy (LSD spec §5 risk #2): only additive changes
// via `ALTER TABLE` in future migration versions. No in-place rewrites.

use rusqlite::Connection;

use super::LogError;

/// DDL applied on every `LauncherSagaLog::open()`. Idempotent:
/// `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS` make
/// reopening the same DB a no-op. Schema mirrors LSD spec §3.2 verbatim
/// (timestamps as RFC3339 TEXT — easier to grep in SQLite shells than
/// epoch ms; PR LSD-2's coordinator wiring serializes via
/// `chrono::DateTime<Utc>::to_rfc3339`).
pub(super) const DDL: &str = "
CREATE TABLE IF NOT EXISTS launcher_saga (
    saga_id        INTEGER PRIMARY KEY,
    name           TEXT NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('running', 'completed', 'failed', 'compensating', 'failed_compensation')),
    started_at     TEXT NOT NULL,
    ended_at       TEXT,
    input_json     TEXT NOT NULL,
    failure_reason TEXT
);

CREATE TABLE IF NOT EXISTS launcher_saga_step (
    saga_id        INTEGER NOT NULL REFERENCES launcher_saga(saga_id) ON DELETE CASCADE,
    step_index     INTEGER NOT NULL,
    name           TEXT NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('pending', 'succeeded', 'failed', 'compensated')),
    cmd_json       TEXT,
    target         TEXT,
    started_at     TEXT NOT NULL,
    ended_at       TEXT,
    output_json    TEXT,
    failure_reason TEXT,
    PRIMARY KEY (saga_id, step_index)
);

CREATE INDEX IF NOT EXISTS idx_launcher_saga_state
    ON launcher_saga(state);
CREATE INDEX IF NOT EXISTS idx_launcher_saga_step_state
    ON launcher_saga_step(saga_id, state);
";

/// Apply `DDL` to a fresh or existing connection.
pub(super) fn run_migrations(conn: &Connection) -> Result<(), LogError> {
    conn.execute_batch(DDL)?;
    Ok(())
}
