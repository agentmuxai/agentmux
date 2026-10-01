// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agent teardown — the ONE path that stops what an agent owns
// (docs/specs/SPEC_AGENT_TEARDOWN_SINGLE_PATH_2026_10_01.md).
//
// Every way of ending an agent calls this with a `Policy`; nothing else in
// srv stops an agent's processes (the structural test below pins it):
//
// | Consumer                                              | Policy            |
// |-------------------------------------------------------|-------------------|
// | pane × / tab × / window-tab × / window ×, ClosePane    | `Policy::close()` |
// |   (`close_pane`, `delete_block`, `delete_tab`,        |                   |
// |   `delete_workspace`; the orphan reaper via close_pane)|                   |
// | `/quit`, `QuitSelf`, no-arg `ClosePane` (`self_quit`)  | `Policy::quit()`  |
// | Stop, `agent.stop`, `FleetBulkStop` Stop               | `Policy::stop(..)`|
//
// Phase 1 moves today's behaviour here unchanged, encoded as policies: the
// close and quit paths are `close_pane::shutdown_one` and `self_quit`'s
// claim/shell steps, verbatim; Stop is `ctrl.stop()`. Later phases change
// what a policy covers (spec §10) without touching the consumers. Controller
// replace and app exit move here in Phase 2.

use serde::Serialize;

use crate::backend::blockcontroller;
use crate::server::agent_resources::{self, AgentIdentity, AgentResources};
use crate::server::AppState;

use super::close_pane::{exit_line, process_line, publish_shutdown, save_final_state, short_command};

/// How the agent's own process is stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliStop {
    /// Stop routing input to it, end the turn, ask it to exit, force-kill at
    /// the deadline, then drop the block's process tracker (which takes what
    /// the agent started) and save its final state (spec §6.2 steps 1, 4–9).
    Graceful,
    /// `controller.stop()` and nothing else: the controller stays registered
    /// and the tracker is kept, so what the agent started keeps running
    /// (today's Stop; spec §5).
    StopOnly { graceful: bool },
}

/// What one teardown covers (spec §5). A consumer picks a constructor; it
/// never adds its own kill code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub cli: CliStop,
    /// Release the agent's work-queue claims (R7) and report the crons that
    /// target it (R8, kept).
    pub release_claims: bool,
    /// Stop the srv-spawned `Shell()` sessions it started (R3).
    pub stop_shell_sessions: bool,
}

impl Policy {
    /// A pane, tab, window-tab or window closing.
    pub const fn close() -> Self {
        Self { cli: CliStop::Graceful, release_claims: false, stop_shell_sessions: false }
    }
    /// `/quit`, `QuitSelf`, no-argument `ClosePane`: everything (spec §0).
    pub const fn quit() -> Self {
        Self { cli: CliStop::Graceful, release_claims: true, stop_shell_sessions: true }
    }
    /// Stop: the CLI only, so background tasks keep running (spec §5).
    pub const fn stop(graceful: bool) -> Self {
        Self { cli: CliStop::StopOnly { graceful }, release_claims: false, stop_shell_sessions: false }
    }
}

/// What a teardown did, for the consumer's summary and the audit.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TeardownReport {
    pub block_id: String,
    /// The agent's id, else its block id.
    pub agent: String,
    pub released_claims: usize,
    pub stopped_shells: usize,
    /// Cron jobs still targeting the agent (kept).
    pub crons_targeting: Vec<String>,
    /// What it owned when the teardown began.
    pub before: AgentResources,
}

/// Tear down every block in `block_ids` concurrently, under ONE deadline:
/// ten agents take one grace period, not ten.
pub async fn run_many(state: &AppState, block_ids: &[String], policy: Policy) -> Vec<TeardownReport> {
    let deadline = std::time::Instant::now() + blockcontroller::SHUTDOWN_GRACE;
    futures_util::future::join_all(block_ids.iter().map(|id| run_one(state, id, policy, deadline))).await
}

/// [`run_many`] for one block.
pub async fn run(state: &AppState, block_id: &str, policy: Policy) -> TeardownReport {
    run_many(state, std::slice::from_ref(&block_id.to_string()), policy)
        .await
        .pop()
        .unwrap_or_default()
}

/// The Stop policy's entry for callers that aren't async: it only calls
/// `controller.stop()` (`Policy::stop`). Errors with `NOT_RUNNING` when the
/// block has no controller, as `agent.stop` always has.
pub fn stop_now(block_id: &str, graceful: bool) -> Result<(), String> {
    debug_assert!(matches!(Policy::stop(graceful).cli, CliStop::StopOnly { .. }));
    let ctrl = blockcontroller::get_controller(block_id)
        .ok_or_else(|| format!("NOT_RUNNING: no controller for block {block_id}"))?;
    ctrl.stop(graceful, blockcontroller::STATUS_DONE)
}

async fn run_one(state: &AppState, block_id: &str, policy: Policy, deadline: std::time::Instant) -> TeardownReport {
    let started = std::time::Instant::now();
    // Identity and inventory while the block is still registered (step 2).
    let who = AgentIdentity::of(state, block_id);
    let before = agent_resources::snapshot(state, block_id);
    let mut report = TeardownReport {
        block_id: block_id.to_string(),
        agent: who.label(block_id),
        before: before.clone(),
        ..Default::default()
    };

    // Step 3: non-process resources, so nothing new reaches a dying agent.
    if policy.release_claims {
        let (released, crons) = release_claims_and_list_crons(state, &who).await;
        report.released_claims = released;
        report.crons_targeting = crons;
    }
    if policy.stop_shell_sessions {
        report.stopped_shells = before
            .shell_sessions
            .iter()
            .filter(|s| state.shell_sessions.stop(&s.shell_id))
            .count();
    }

    match policy.cli {
        CliStop::StopOnly { graceful } => {
            if let Err(e) = stop_now(block_id, graceful) {
                tracing::debug!(block_id = %block_id, error = %e, "agent_teardown: stop");
            }
        }
        CliStop::Graceful => close_cli(state, block_id, &who, &before, deadline, started).await,
    }
    report
}

/// Today's per-agent close (`SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md`
/// §4.2), moved here from `close_pane::shutdown_one` unchanged.
async fn close_cli(
    state: &AppState,
    block_id: &str,
    who: &AgentIdentity,
    before: &AgentResources,
    deadline: std::time::Instant,
    started: std::time::Instant,
) {
    let label = if who.agent_id.is_empty() { "agent".to_string() } else { who.agent_id.clone() };
    // 1. Stop routing input: no respawn (resync_controller refuses), no
    //    jekt/muxbus delivery, and — once out of CONTROLLER_REGISTRY — no
    //    AgentInput either.
    blockcontroller::mark_closing(block_id);
    state.reactive_handler.unregister_block(block_id);
    let ctrl = blockcontroller::take_controller(block_id);
    // 2–4. End the turn, ask the process to exit, force-kill at the deadline;
    //      resolves once it has actually exited.
    let (controller_type, outcome) = match ctrl {
        Some(ctrl) => {
            if ctrl.get_runtime_status().turn_active {
                publish_shutdown(state, block_id, "interrupt", "turn interrupted".into(), serde_json::json!({}));
            }
            let outcome = ctrl.shutdown(deadline).await;
            (ctrl.controller_type().to_string(), outcome)
        }
        None => (String::new(), blockcontroller::StopOutcome::NotRunning),
    };
    let (step, text, extra) = exit_line(&label, &outcome, started.elapsed());
    publish_shutdown(state, block_id, step, text, extra);
    // 5. Only now drop the process tracker — anything the agent itself
    //    started dies with it, but the agent got to exit first. Whatever is
    //    still alive at this point is what the release kills.
    let alive: std::collections::HashSet<u32> =
        state.process_tracker.list_block(block_id).into_iter().map(|p| p.pid).collect();
    blockcontroller::release_block_processes(block_id);
    for p in &before.processes {
        let killed = alive.contains(&p.pid);
        publish_shutdown(
            state,
            block_id,
            "process",
            process_line(&p.command, p.pid, killed),
            serde_json::json!({ "process": {
                "pid": p.pid,
                "name": short_command(&p.command),
                "outcome": if killed { "killed" } else { "stopped" },
            } }),
        );
    }
    blockcontroller::mark_closing_stopped(block_id);
    crate::backend::container_credential::revoke_block(block_id);
    // 6. Save final state.
    save_final_state(state, block_id);
    publish_shutdown(state, block_id, "saved", "conversation saved".into(), serde_json::json!({}));
    publish_shutdown(state, block_id, "done", "closing".into(), serde_json::json!({}));
    tracing::info!(
        block_id = %block_id,
        controller_type = %controller_type,
        outcome = outcome.as_str(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "agent_shutdown"
    );
}

/// Release the agent's work claims, so the items go back to the pool now
/// rather than when the 120 s lease expires, and list the crons that target
/// it (kept: a cron is a deliberate schedule that resumes when the agent is
/// reopened). SQLite reads and writes, off the async workers.
async fn release_claims_and_list_crons(state: &AppState, who: &AgentIdentity) -> (usize, Vec<String>) {
    let (identity_store, shared_store) = (state.identity_store.clone(), state.shared_store.clone());
    let (a, u) = (who.agent_id.clone(), who.uid.clone());
    let result = tokio::task::spawn_blocking(move || {
        let now = agentmux_common::time::now_ms();
        let items = identity_store.work_queue_claimed_by(&a, u.as_deref().unwrap_or("")).unwrap_or_default();
        let released = agent_resources::claims_held_by(&items, &a, u.as_deref())
            .into_iter()
            .filter(|w| {
                matches!(
                    identity_store.work_queue_release(&w.id, &w.claimed_by, &w.claimed_by_uid, w.attempts, "agent quit", now),
                    Ok(Some(_))
                )
            })
            .count();
        let crons = shared_store
            .and_then(|s| s.cron_list().ok())
            .map(|jobs| agent_resources::crons_targeting(&jobs, &a, u.as_deref()))
            .unwrap_or_default();
        (released, crons)
    })
    .await
    .unwrap_or_default();
    if result.0 > 0 {
        crate::server::work_queue::publish_changed(state);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies_encode_todays_scopes() {
        assert_eq!(Policy::close(), Policy { cli: CliStop::Graceful, release_claims: false, stop_shell_sessions: false });
        assert_eq!(Policy::quit(), Policy { cli: CliStop::Graceful, release_claims: true, stop_shell_sessions: true });
        assert_eq!(Policy::stop(false).cli, CliStop::StopOnly { graceful: false });
        assert!(!Policy::stop(true).release_claims && !Policy::stop(true).stop_shell_sessions);
    }

    /// The single path (spec §3 G1, §9.1): outside this module and the
    /// process machinery itself, nothing in srv stops an agent's processes.
    #[test]
    fn nothing_else_stops_an_agent() {
        // (file suffix, why it may) — every entry is a definition, the
        // machinery's own internals, or a phase-2 consumer named in the spec.
        const ALLOWED: &[(&str, &str)] = &[
            ("sagas/agent_teardown.rs", "the one path"),
            ("backend/blockcontroller/", "controller and tracker internals; controller replace moves here in Phase 2"),
            ("backend/process_tracker/", "the tracker itself"),
            ("backend/shell_node.rs", "ShellSessionRegistry's own stop/stop_all"),
            ("backend/identity_spawn.rs", "test fixture"),
            ("server/http_shell.rs", "the ShellStop tool: the agent stopping one shell on purpose"),
            ("main.rs", "app exit: moves here in Phase 2 (spec §10)"),
        ];
        const PATTERNS: &[&str] = &[
            "release_block_processes(",
            "shell_sessions.stop(",
            ".stop_all()",
            ".shutdown(deadline",
            "ctrl.stop(",
        ];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
                if ALLOWED.iter().any(|(a, _)| rel.starts_with(a) || rel == *a) {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                // Code only: skip comment lines and everything from a test module on.
                let code = text.split("#[cfg(test)]").next().unwrap_or("");
                for (i, line) in code.lines().enumerate() {
                    let t = line.trim_start();
                    if t.starts_with("//") {
                        continue;
                    }
                    if let Some(p) = PATTERNS.iter().find(|p| line.contains(*p)) {
                        offenders.push(format!("{rel}:{}: `{p}`", i + 1));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "an agent's processes are stopped only by sagas::agent_teardown (spec SPEC_AGENT_TEARDOWN_SINGLE_PATH §3 G1); \
             route these through it:\n{}",
            offenders.join("\n")
        );
    }
}
