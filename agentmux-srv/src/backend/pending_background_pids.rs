// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-memory holding pen for a declared-background task's OS pid when it
//! arrives before `db_background_tasks` has a row to attach it to.
//!
//! `agentmux-bashwrap` publishes its own pid over WPS essentially at process
//! start (before the wrapped command even begins running) — see
//! `bash_wrap.rs`. The `db_background_tasks` row, by contrast, is only
//! created once the FRONTEND observes the tool call as an accepted
//! background launch (requires the literal "Command running in background
//! with ID:" tool-result prefix) and pushes `docknodestatus`, which is a
//! full extra round-trip through the CLI's own stream-json processing. The
//! pid publish routinely wins that race.
//!
//! `background_task_set_pid` (`background_tasks.rs`) is a bare
//! `UPDATE ... WHERE id = ?` — it silently no-ops (`Ok(false)`) when the
//! row doesn't exist yet, and the pid would be lost forever with no
//! retry that could ever succeed (retrying the same too-early write
//! doesn't help — the row still won't exist). This module closes that
//! gap: a pid that arrives too early is stashed here instead, and applied
//! the moment `background_task_observe` creates the row. See
//! docs/specs/SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md and the
//! Codex/reagentx findings on PR #2681 this was written to address.

use std::collections::HashMap;

use parking_lot::Mutex;

/// How long a stashed pid waits for its row before being dropped. Generous
/// relative to the observed race window (a network round-trip plus one
/// SQLite write, normally well under a second) but still bounded — a
/// declared-background call whose launch is never actually accepted as
/// background client-side (e.g. the command finished fast enough that
/// Claude's harness returned synchronously despite `run_in_background:
/// true` — see issue #2519) would otherwise leak an entry here forever,
/// since `background_task_observe` would then never fire for that id.
const PENDING_PID_TTL_MS: i64 = 5 * 60 * 1000;

struct PendingEntry {
    pid: i64,
    stashed_at_ms: i64,
}

#[derive(Default)]
pub struct PendingBackgroundPids {
    inner: Mutex<HashMap<String, PendingEntry>>,
}

impl PendingBackgroundPids {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a pid that arrived before its `db_background_tasks` row
    /// existed. Overwrites any previous stash for the same id (only the
    /// latest pid is meaningful — there is at most one live process per
    /// declared-background task id).
    pub fn stash(&self, id: &str, pid: i64, now_ms: i64) {
        let mut guard = self.inner.lock();
        guard.retain(|_, e| now_ms.saturating_sub(e.stashed_at_ms) < PENDING_PID_TTL_MS);
        guard.insert(id.to_string(), PendingEntry { pid, stashed_at_ms: now_ms });
    }

    /// Remove and return a stashed pid for `id`, if one is still pending
    /// and hasn't expired. Called right after `background_task_observe`
    /// creates/refreshes a row, so a pid that arrived first can be applied
    /// immediately.
    pub fn take(&self, id: &str, now_ms: i64) -> Option<i64> {
        let mut guard = self.inner.lock();
        let entry = guard.remove(id)?;
        if now_ms.saturating_sub(entry.stashed_at_ms) >= PENDING_PID_TTL_MS {
            return None;
        }
        Some(entry.pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_on_unknown_id_is_none() {
        let p = PendingBackgroundPids::new();
        assert_eq!(p.take("unknown", 1000), None);
    }

    #[test]
    fn stash_then_take_round_trips_and_consumes() {
        let p = PendingBackgroundPids::new();
        p.stash("t1", 4242, 1000);
        assert_eq!(p.take("t1", 1500), Some(4242));
        // Consumed — a second take finds nothing.
        assert_eq!(p.take("t1", 1500), None);
    }

    #[test]
    fn take_past_ttl_returns_none_and_still_consumes_the_entry() {
        let p = PendingBackgroundPids::new();
        p.stash("t1", 4242, 1000);
        assert_eq!(p.take("t1", 1000 + PENDING_PID_TTL_MS), None);
        // Even a fresh-enough take afterward finds nothing — it's gone.
        assert_eq!(p.take("t1", 1000 + PENDING_PID_TTL_MS), None);
    }

    #[test]
    fn stash_overwrites_a_previous_pending_pid_for_the_same_id() {
        let p = PendingBackgroundPids::new();
        p.stash("t1", 111, 1000);
        p.stash("t1", 222, 1001);
        assert_eq!(p.take("t1", 1002), Some(222));
    }

    #[test]
    fn stash_evicts_other_expired_entries_as_a_side_effect() {
        let p = PendingBackgroundPids::new();
        p.stash("old", 1, 1000);
        // Well past TTL by the time this second stash happens.
        p.stash("new", 2, 1000 + PENDING_PID_TTL_MS + 1);
        assert_eq!(p.take("old", 1000 + PENDING_PID_TTL_MS + 1), None);
        assert_eq!(p.take("new", 1000 + PENDING_PID_TTL_MS + 1), Some(2));
    }

    #[test]
    fn different_ids_coexist_independently() {
        let p = PendingBackgroundPids::new();
        p.stash("t1", 1, 1000);
        p.stash("t2", 2, 1000);
        assert_eq!(p.take("t1", 1000), Some(1));
        assert_eq!(p.take("t2", 1000), Some(2));
    }
}
