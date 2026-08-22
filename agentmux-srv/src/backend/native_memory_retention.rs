// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Retention/GC for `db_agent_native_memory_versions` — closes §7.1 of
//! docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md,
//! left open in v1 ("no pruning is proposed... flag if unbounded growth is
//! a concern"). Operator-confirmed policy (2026-08-22): hybrid age +
//! min-count floor — a version is pruned only once it is BOTH older than
//! [`MAX_AGE_MS`] AND ranked beyond the [`MIN_KEEP`] most-recent versions
//! for its `(agent_id, filename)`. The floor exists so an agent that
//! rarely touches a given memory file never loses its entire history just
//! because every version happens to be old; the age bound exists so a
//! hyperactive agent's frequently-rewritten file doesn't grow unbounded
//! just because every version happens to be recent. See
//! `agent_native_memory_versions.rs::agent_native_memory_version_prune`
//! for the actual query.
//!
//! Runs as its own periodic background sweep, started once at server
//! startup alongside `native_memory_drift`'s detection sweeps (a sibling
//! concern, not the same one — drift *detects* out-of-band writes,
//! retention *prunes* old versions regardless of source). Deliberately a
//! much slower cadence than drift detection's 30s: pruning is housekeeping,
//! not latency-sensitive — nothing is waiting on a stale version's removal
//! the way a reviewer is waiting to see a drifted write show up.

use std::sync::Arc;
use std::time::Duration;

use crate::backend::storage::store::Store;

/// Pruning is housekeeping, not latency-sensitive — once a day is generous
/// enough that growth stays bounded without adding meaningful DB load.
const SWEEP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Keep at least this many most-recent versions per `(agent_id, filename)`,
/// regardless of age.
pub(crate) const MIN_KEEP: u32 = 50;

/// Beyond `MIN_KEEP`, prune versions older than this.
pub(crate) const MAX_AGE_MS: i64 = 90 * 24 * 60 * 60 * 1000;

/// One pass: prune every `(agent_id, filename)` pair that has any recorded
/// version history. Returns the number of version rows deleted (for
/// logging; tests assert on it directly). A failure listing targets or
/// pruning one file is logged and does not stop the sweep from covering
/// everything else — matches `native_memory_drift`'s own per-item error
/// handling.
pub(crate) fn prune_once(id_store: &Store) -> usize {
    let targets = match id_store.agent_native_memory_version_list_distinct_files() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "native_memory_retention: failed to list files needing pruning");
            return 0;
        }
    };

    let mut pruned = 0;
    for (agent_id, filename) in targets {
        match id_store.agent_native_memory_version_prune(&agent_id, &filename, MIN_KEEP, MAX_AGE_MS) {
            Ok(0) => {}
            Ok(n) => {
                pruned += n;
                tracing::info!(agent_id, filename, pruned = n, "native_memory_retention: pruned old versions");
            }
            Err(e) => tracing::warn!(agent_id, filename, error = %e, "native_memory_retention: prune failed"),
        }
    }
    pruned
}

/// Start the periodic sweep. Call once at server startup with the store
/// version history is recorded in. Returns immediately — runs as a
/// spawned background task for the process lifetime (no shutdown handle;
/// srv itself owns the process lifetime, same as `native_memory_drift::spawn`).
pub fn spawn(id_store: Arc<Store>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            tick.tick().await;
            prune_once(&id_store);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn shared_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open_shared(tmp.path()).unwrap()
    }

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    fn backdate(store: &Store, version_id: &str, days_ago: i64) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let conn = store.conn().lock().unwrap();
        conn.execute(
            "UPDATE db_agent_native_memory_versions SET created_at = ?1 WHERE id = ?2",
            params![now_ms - days_ago * DAY_MS, version_id],
        )
        .unwrap();
    }

    #[test]
    fn prune_once_covers_every_file_with_recorded_history() {
        let store = shared_store();
        for (agent_id, filename) in [("agent-1", "a.md"), ("agent-1", "b.md"), ("agent-2", "a.md")] {
            for i in 0..(MIN_KEEP + 3) {
                let v = store
                    .agent_native_memory_version_insert(agent_id, filename, &format!("v{i}"), "human", "{}", "")
                    .unwrap();
                backdate(&store, &v.id, 200); // past MAX_AGE_MS
            }
        }

        let pruned = prune_once(&store);
        assert_eq!(pruned, 3 * 3, "3 excess versions per file, across 3 distinct files");

        for (agent_id, filename) in [("agent-1", "a.md"), ("agent-1", "b.md"), ("agent-2", "a.md")] {
            assert_eq!(
                store.agent_native_memory_version_list(agent_id, filename).unwrap().len(),
                MIN_KEEP as usize
            );
        }
    }

    #[test]
    fn prune_once_is_a_no_op_when_nothing_qualifies() {
        let store = shared_store();
        store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "v1", "human", "{}", "")
            .unwrap();
        assert_eq!(prune_once(&store), 0);
        assert_eq!(store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap().len(), 1);
    }

    #[test]
    fn prune_once_returns_zero_when_no_version_history_exists_at_all() {
        let store = shared_store();
        assert_eq!(prune_once(&store), 0);
    }
}
