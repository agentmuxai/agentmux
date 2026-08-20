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
//! row doesn't exist yet, and the pid would be lost forever with no retry
//! that could ever succeed (retrying the same too-early write doesn't
//! help — the row still won't exist).
//!
//! [`set_or_stash`](PendingBackgroundPids::set_or_stash) and
//! [`observe_and_apply`](PendingBackgroundPids::observe_and_apply) are the
//! two sides of the fix, called from `websocket.rs`'s
//! `COMMAND_BACKGROUND_TASK_PID` and `COMMAND_DOCK_NODE_STATUS` handlers
//! respectively. Both run their DB call (via the `try_set`/`apply` closure)
//! WHILE HOLDING this module's own lock, not as a separate step after it —
//! an earlier version of this module exposed plain `stash`/`take` methods
//! instead, with each handler doing its own DB call first and the
//! stash/take second, unlocked in between. That let the two handlers'
//! steps interleave (pid handler's DB call sees no row → before it stashes,
//! the observe handler's own take() already ran and found nothing →
//! observe handler's row now exists but nothing will ever re-check it →
//! pid handler's stash lands after the one take() that would have
//! consumed it, and sits unclaimed until the TTL drops it). Making the DB
//! call part of the SAME critical section as the buffer mutation closes
//! that window: whichever handler's critical section runs second always
//! observes the first one's effect, DB row or stash, correctly. See
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

    fn evict_expired_locked(guard: &mut HashMap<String, PendingEntry>, now_ms: i64) {
        guard.retain(|_, e| now_ms.saturating_sub(e.stashed_at_ms) < PENDING_PID_TTL_MS);
    }

    /// Called from the pid-arrival side (`COMMAND_BACKGROUND_TASK_PID`).
    /// Runs `try_set(pid)` (the `background_task_set_pid` DB call) while
    /// holding this module's lock; if it reports the row doesn't exist yet
    /// (`Ok(false)`), stashes the pid instead of losing it. Atomic with
    /// `observe_and_apply` below for the same `id` — see the module doc
    /// comment for why that matters.
    pub fn set_or_stash<E>(
        &self,
        id: &str,
        pid: i64,
        now_ms: i64,
        try_set: impl FnOnce(i64) -> Result<bool, E>,
    ) -> Result<(), E> {
        let mut guard = self.inner.lock();
        Self::evict_expired_locked(&mut guard, now_ms);
        match try_set(pid)? {
            true => {
                guard.remove(id);
            }
            false => {
                guard.insert(id.to_string(), PendingEntry { pid, stashed_at_ms: now_ms });
            }
        }
        Ok(())
    }

    /// Called from the row-creation side (`COMMAND_DOCK_NODE_STATUS`),
    /// after its own `background_task_observe(...)` call. Checks for a
    /// pid stashed ahead of the row's existence and, if present and not
    /// expired, applies it via `apply` (another `background_task_set_pid`
    /// call) while holding this module's lock. Atomic with `set_or_stash`
    /// above for the same `id`.
    pub fn observe_and_apply<E>(
        &self,
        id: &str,
        now_ms: i64,
        apply: impl FnOnce(i64) -> Result<bool, E>,
    ) -> Result<(), E> {
        let mut guard = self.inner.lock();
        Self::evict_expired_locked(&mut guard, now_ms);
        if let Some(entry) = guard.remove(id) {
            apply(entry.pid)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct TestErr;

    #[test]
    fn set_or_stash_applies_directly_when_the_row_already_exists() {
        let p = PendingBackgroundPids::new();
        let mut set_calls = Vec::new();
        let r: Result<(), TestErr> = p.set_or_stash("t1", 4242, 1000, |pid| {
            set_calls.push(pid);
            Ok(true) // row exists, update succeeded
        });
        assert!(r.is_ok());
        assert_eq!(set_calls, vec![4242]);
        // Nothing stashed — a later observe_and_apply finds nothing to apply.
        let mut applied = Vec::new();
        let _: Result<(), TestErr> = p.observe_and_apply("t1", 1001, |pid| {
            applied.push(pid);
            Ok(true)
        });
        assert!(applied.is_empty());
    }

    #[test]
    fn set_or_stash_then_observe_and_apply_delivers_the_pid() {
        // The exact race this module exists to close: pid arrives before
        // the row does.
        let p = PendingBackgroundPids::new();
        let r: Result<(), TestErr> = p.set_or_stash("t1", 4242, 1000, |_pid| Ok(false)); // no row yet
        assert!(r.is_ok());

        let mut applied = Vec::new();
        let r2: Result<(), TestErr> = p.observe_and_apply("t1", 1050, |pid| {
            applied.push(pid);
            Ok(true)
        });
        assert!(r2.is_ok());
        assert_eq!(applied, vec![4242]);

        // Consumed — a second observe_and_apply finds nothing.
        let mut applied2 = Vec::new();
        let _: Result<(), TestErr> = p.observe_and_apply("t1", 1051, |pid| {
            applied2.push(pid);
            Ok(true)
        });
        assert!(applied2.is_empty());
    }

    #[test]
    fn observe_and_apply_before_any_pid_arrives_is_a_no_op() {
        let p = PendingBackgroundPids::new();
        let mut applied = Vec::new();
        let _: Result<(), TestErr> = p.observe_and_apply("t1", 1000, |pid| {
            applied.push(pid);
            Ok(true)
        });
        assert!(applied.is_empty());
    }

    #[test]
    fn set_or_stash_propagates_the_db_error_without_stashing() {
        let p = PendingBackgroundPids::new();
        let r: Result<(), TestErr> = p.set_or_stash("t1", 4242, 1000, |_pid| Err(TestErr));
        assert_eq!(r, Err(TestErr));
        // Nothing stashed despite the error.
        let mut applied = Vec::new();
        let _: Result<(), TestErr> = p.observe_and_apply("t1", 1001, |pid| {
            applied.push(pid);
            Ok(true)
        });
        assert!(applied.is_empty());
    }

    #[test]
    fn observe_and_apply_propagates_the_db_error_but_still_consumes_the_stash() {
        // Matches the module's stated semantics: this is a best-effort
        // relay, not something worth retrying indefinitely — a stashed
        // pid is consumed on the attempt regardless of outcome.
        let p = PendingBackgroundPids::new();
        let _: Result<(), TestErr> = p.set_or_stash("t1", 4242, 1000, |_pid| Ok(false));
        let r: Result<(), TestErr> = p.observe_and_apply("t1", 1001, |_pid| Err(TestErr));
        assert_eq!(r, Err(TestErr));
        let mut applied = Vec::new();
        let _: Result<(), TestErr> = p.observe_and_apply("t1", 1002, |pid| {
            applied.push(pid);
            Ok(true)
        });
        assert!(applied.is_empty());
    }

    #[test]
    fn a_stash_past_ttl_is_not_applied() {
        let p = PendingBackgroundPids::new();
        let _: Result<(), TestErr> = p.set_or_stash("t1", 4242, 1000, |_pid| Ok(false));
        let mut applied = Vec::new();
        let _: Result<(), TestErr> = p.observe_and_apply("t1", 1000 + PENDING_PID_TTL_MS, |pid| {
            applied.push(pid);
            Ok(true)
        });
        assert!(applied.is_empty());
    }

    #[test]
    fn set_or_stash_overwrites_a_previous_pending_pid_for_the_same_id() {
        let p = PendingBackgroundPids::new();
        let _: Result<(), TestErr> = p.set_or_stash("t1", 111, 1000, |_pid| Ok(false));
        let _: Result<(), TestErr> = p.set_or_stash("t1", 222, 1001, |_pid| Ok(false));
        let mut applied = Vec::new();
        let _: Result<(), TestErr> = p.observe_and_apply("t1", 1002, |pid| {
            applied.push(pid);
            Ok(true)
        });
        assert_eq!(applied, vec![222]);
    }

    #[test]
    fn different_ids_coexist_independently() {
        let p = PendingBackgroundPids::new();
        let _: Result<(), TestErr> = p.set_or_stash("t1", 1, 1000, |_pid| Ok(false));
        let _: Result<(), TestErr> = p.set_or_stash("t2", 2, 1000, |_pid| Ok(false));
        let mut applied = Vec::new();
        let _: Result<(), TestErr> = p.observe_and_apply("t1", 1000, |pid| {
            applied.push(pid);
            Ok(true)
        });
        let _: Result<(), TestErr> = p.observe_and_apply("t2", 1000, |pid| {
            applied.push(pid);
            Ok(true)
        });
        assert_eq!(applied, vec![1, 2]);
    }
}
