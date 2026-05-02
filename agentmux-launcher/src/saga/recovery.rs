// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// LSD-3 — startup recovery walker for unresolved launcher sagas.
//
// Spec: `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` §3.5.
// Companion plan: `docs/specs/SAGA_ARCHITECTURE_EXECUTION_PLAN_2026-05-01.md` §2 batch 3.
//
// Called from `main.rs::run_windows` AFTER opening `LauncherSagaLog`
// and BEFORE spawning the saga coordinator. The walker:
//   1. Reads `saga_log.unresolved_sagas()` (sagas in `running` /
//      `compensating` / `failed` state — see LSD spec §3.3).
//   2. For each unresolved saga, calls
//      `saga_log.mark_failed_compensation(saga_id, ..reason..)` with
//      a human-readable reason that names the prior state.
//   3. Logs a `[saga-recovery]` warning per saga (id + name + state)
//      and an info line with the total count.
//
// **Critical: we DO NOT auto-replay or auto-compensate launcher sagas.**
// LSD spec §3.5 — launcher sagas drive host-side effects on live OS
// state (pane reaps, pool drains) that may already be partially
// applied. Re-issuing the forward command might double-act; deriving
// an inverse without pre-state is unsound. The right escape hatch is
// operator review via `--diag sagas`. The recovery walker's job is to
// move the row from "in-flight at restart" to a clearly-marked
// terminal state so the operator can see it and decide.
//
// Compare with srv's recovery (`agentmux-srv/src/sagas/recovery.rs`):
// srv DOES drive inverse-command compensation through the reducer,
// because srv saga effects are reducer-level state mutations the
// reducer can undo. Launcher sagas don't have that property — they
// dispatch real-world host commands. Hence the asymmetry.
//
// The walker is idempotent: `mark_failed_compensation` is itself
// idempotent (LSD-1 PR), and `unresolved_sagas` only returns rows
// not already in `failed_compensation`. A second crash mid-recovery
// just re-touches the same rows on the next restart with the same
// reason string.

use std::sync::Arc;

use super::LauncherSagaLog;

/// Walk all unresolved sagas in the durable log and mark each as
/// `failed_compensation`. Returns the count of sagas touched.
///
/// Errors propagate from `LauncherSagaLog::unresolved_sagas` (read
/// failure: corrupt SQLite, schema mismatch). A failure here should
/// be fatal at the caller — without recovery, sagas left over from a
/// prior crash stay in `running` state forever and the next restart
/// would do the same dance.
///
/// Per-saga `mark_failed_compensation` failures are logged but NOT
/// fatal — the walker continues to subsequent sagas. Stopping on one
/// row's write failure would leave later unresolved sagas in `running`
/// when we could have cleaned them. Operators see the per-saga error
/// in the launcher log.
pub async fn compensate_unresolved_launcher_sagas(
    saga_log: &Arc<LauncherSagaLog>,
) -> Result<usize, String> {
    let unresolved = saga_log
        .unresolved_sagas()
        .map_err(|e| format!("read unresolved sagas: {}", e))?;

    if unresolved.is_empty() {
        crate::log("[saga-recovery] no unresolved sagas at startup");
        return Ok(0);
    }

    let total = unresolved.len();
    crate::log(&format!(
        "[saga-recovery] found {} unresolved saga(s) at startup — marking failed_compensation",
        total
    ));

    let mut touched = 0usize;
    for saga in &unresolved {
        let reason = format!(
            "launcher restarted while saga in state '{}'",
            saga.state
        );
        crate::log(&format!(
            "[saga-recovery] WARN saga {} ({}) was '{}' when launcher last exited; marking failed_compensation",
            saga.saga_id, saga.name, saga.state
        ));
        match saga_log.mark_failed_compensation(saga.saga_id, &reason) {
            Ok(()) => {
                touched += 1;
            }
            Err(e) => {
                // Best-effort: log + keep walking. A single SQLite
                // write hiccup shouldn't block the rest of the cleanup.
                crate::log(&format!(
                    "[saga-recovery] WARN saga {} mark_failed_compensation failed: {} — leaving in '{}'",
                    saga.saga_id, e, saga.state
                ));
            }
        }
    }

    crate::log(&format!(
        "[saga-recovery] processed {}/{} unresolved sagas",
        touched, total
    ));
    Ok(touched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saga::PipeTarget;
    use agentmux_common::ipc::Command;

    /// Integration: simulate a "crashed" saga by writing a `running`
    /// row directly via `LauncherSagaLog`. Run the recovery walker.
    /// Assert the saga is now `failed_compensation`.
    #[tokio::test]
    async fn recovery_walker_marks_running_saga_failed_compensation() {
        let log = Arc::new(LauncherSagaLog::open_in_memory().unwrap());
        // Crashed saga: started + one step dispatched, never terminated.
        log.start_saga(11, "window_cleanup_cascade", &serde_json::json!({"label": "win-3"}))
            .unwrap();
        log.start_step(
            11,
            0,
            "issue_cmd_host_reap_panes",
            PipeTarget::Host,
            &Command::Ping { nonce: 0 },
        )
        .unwrap();

        let touched = compensate_unresolved_launcher_sagas(&log).await.unwrap();
        assert_eq!(touched, 1);

        // Saga has moved to failed_compensation; no longer unresolved.
        let unresolved = log.unresolved_sagas().unwrap();
        assert!(
            unresolved.is_empty(),
            "saga 11 should no longer be unresolved, got: {:?}",
            unresolved
        );
        let snap = log.snapshot_recent(10).unwrap();
        let saga = snap.iter().find(|s| s.saga_id == 11).unwrap();
        assert_eq!(saga.state, "failed_compensation");
        assert!(
            saga.failure_reason
                .as_deref()
                .unwrap_or("")
                .contains("launcher restarted while saga in state 'running'"),
            "expected failure_reason to name the prior state, got: {:?}",
            saga.failure_reason
        );
    }

    /// Recovery on an empty log is a no-op + returns 0.
    #[tokio::test]
    async fn recovery_walker_empty_log_returns_zero() {
        let log = Arc::new(LauncherSagaLog::open_in_memory().unwrap());
        let touched = compensate_unresolved_launcher_sagas(&log).await.unwrap();
        assert_eq!(touched, 0);
    }

    /// Recovery walker is idempotent: calling it twice doesn't error
    /// and doesn't re-touch sagas already in `failed_compensation`.
    /// Mirrors srv's recovery idempotence guarantee.
    #[tokio::test]
    async fn recovery_walker_is_idempotent_across_repeated_calls() {
        let log = Arc::new(LauncherSagaLog::open_in_memory().unwrap());
        log.start_saga(7, "saga_a", &serde_json::json!({})).unwrap();

        let first = compensate_unresolved_launcher_sagas(&log).await.unwrap();
        assert_eq!(first, 1);

        // Second call — saga 7 is now failed_compensation, NOT
        // unresolved, so the walker touches 0.
        let second = compensate_unresolved_launcher_sagas(&log).await.unwrap();
        assert_eq!(second, 0);
    }

    /// Multiple unresolved sagas → all get marked, return value is the
    /// total touched count.
    #[tokio::test]
    async fn recovery_walker_handles_multiple_unresolved_sagas() {
        let log = Arc::new(LauncherSagaLog::open_in_memory().unwrap());
        log.start_saga(1, "saga_a", &serde_json::json!({})).unwrap();
        log.start_saga(2, "saga_b", &serde_json::json!({"x": 2})).unwrap();
        log.start_saga(3, "saga_c", &serde_json::json!({})).unwrap();
        // Saga 3 is also failed (still surfaces via unresolved_sagas
        // per LSD-1 + spec §3.5 — recovery upgrades it too).
        log.terminate_saga(
            3,
            super::super::log::SagaOutcome::Failed {
                reason: "evicted".into(),
            },
        )
        .unwrap();

        let touched = compensate_unresolved_launcher_sagas(&log).await.unwrap();
        assert_eq!(touched, 3);

        let unresolved = log.unresolved_sagas().unwrap();
        assert!(unresolved.is_empty(), "all sagas resolved post-walk");
    }
}
