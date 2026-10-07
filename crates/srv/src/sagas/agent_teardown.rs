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
// | Stop, `agent.stop`, `FleetBulkStop` Stop, the watchdog | `Policy::stop(..)`|
// | Controller replace (`resync_controller`)              | `Policy::replace()` |
// | App exit (`main.rs`)                                  | `Policy::app_exit()` |
// | A drawer closed, a `/btw` throwaway, a spawn rollback, | `discard`         |
// |   the backstop after a close                          |                   |
//
// `Shell()` and `!cmd` processes join the agent's tracker when they spawn
// (`track_adopted`), and a graceful teardown closes the agent's PtyShell
// drawers first, so all of them end with it.
//
// Phase 1 moved today's behaviour here unchanged, encoded as policies: the
// close and quit paths are the former `close_pane::shutdown_one` and
// `self_quit`'s claim/shell steps, verbatim; Stop is `ctrl.stop()`. Phase 2 adds controller
// replace (`stop_for_replace`, unchanged) and app exit, which now closes every
// agent gracefully instead of leaving them to the launcher's backstop. Later
// phases change what a policy covers (spec §10) without touching the consumers.

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
    /// The controller is being replaced by a new one for the same block
    /// (session restart): only its own CLI process dies; the tracker and
    /// every declared-background task survive and are adopted by the new
    /// controller (`SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md`).
    Replace,
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
    /// Stop its declared `run_in_background` tasks (R2) explicitly. A
    /// graceful close takes them anyway with the tracker; this is for Stop,
    /// which keeps the tracker (spec §5, §7.4).
    pub stop_background: bool,
    /// Stop a container agent's container (R6), which also drops its
    /// dev-proxy routes. Stopped, not removed: the next launch restarts it
    /// (`ContainerManager::ensure_running`), and its volume keeps its state.
    pub stop_container: bool,
}

impl Policy {
    /// A pane, tab, window-tab or window closing: the agent and everything
    /// it runs, `Shell()` sessions included; claims stay until their lease
    /// expires, as before (spec §5).
    pub const fn close() -> Self {
        Self { cli: CliStop::Graceful, release_claims: false, stop_shell_sessions: true, stop_background: false, stop_container: true }
    }
    /// `/quit`, `QuitSelf`, no-argument `ClosePane`: everything (spec §0).
    pub const fn quit() -> Self {
        Self { cli: CliStop::Graceful, release_claims: true, stop_shell_sessions: true, stop_background: false, stop_container: true }
    }
    /// Stop (`agent.stop`, `FleetBulkStop`): the CLI, and its background
    /// tasks only if asked; its claims go back to the pool (spec §5). The
    /// block, its `Shell()` sessions and drawers stay.
    pub const fn stop(graceful: bool, stop_background: bool) -> Self {
        Self { cli: CliStop::StopOnly { graceful }, release_claims: true, stop_shell_sessions: false, stop_background, stop_container: false }
    }
    /// The watchdog's max-runtime / idle stop: the CLI only, as it always
    /// has been. Its claims stay, since it may be restarted on the next
    /// message.
    pub const fn stop_cli_only(graceful: bool) -> Self {
        Self { cli: CliStop::StopOnly { graceful }, release_claims: false, stop_shell_sessions: false, stop_background: false, stop_container: false }
    }
    /// Restart / controller replace: the CLI only; background tasks survive.
    pub const fn replace() -> Self {
        Self { cli: CliStop::Replace, release_claims: false, stop_shell_sessions: false, stop_background: false, stop_container: false }
    }
    /// App exit: every agent closes gracefully and its `Shell()` sessions
    /// stop. Claims stay: the agent comes back with the app and its lease
    /// covers the gap (spec §5).
    pub const fn app_exit() -> Self {
        Self { cli: CliStop::Graceful, release_claims: false, stop_shell_sessions: true, stop_background: false, stop_container: true }
    }
}

/// Setting: does Stop keep the agent's `run_in_background` tasks (dev
/// servers) running when the caller doesn't say? Per user, default `true`,
/// today's behaviour (spec §11 O2).
pub const SETTING_STOP_KEEPS_BACKGROUND: &str = "agent:stopkeepsbackground";

/// App exit's overall cap (spec §11 O3), the leftover-shell sweep included:
/// every agent closes concurrently under it. The launcher waits longer than
/// this for srv to exit on every quit (`SRV_EXIT_WAIT`, `quiesce_srv`), so
/// it force-kills srv only if the teardown overran.
pub const APP_EXIT_CAP: std::time::Duration = agentmux_common::process::SRV_APP_EXIT_CAP;

/// The grace the leftover-shell sweep gives its kill tasks. [reagent #1422 P2]
const SHELL_SWEEP_GRACE: std::time::Duration = std::time::Duration::from_millis(800);

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
    /// Background tasks stopped explicitly (Stop with `stop_background`).
    pub stopped_background: usize,
    /// Stop only: the CLI stop failed. Nothing else was touched (claims and
    /// background tasks stay), and [`stop`] returns this as its error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_error: Option<String>,
    /// The container agent's container that was stopped, if any.
    pub stopped_container: Option<String>,
    /// Processes from `before` still running after the teardown and one more
    /// forced kill each (spec §6.2 step 8). Empty is the goal.
    pub survivors: Vec<agent_resources::ProcessEntry>,
    /// What it owned when the teardown began.
    pub before: AgentResources,
}

/// Every teardown and window close in flight. Each runs as its own task, so
/// a caller that stops waiting can't cut it short: an HTTP handler whose
/// client hung up (the host gives `CloseWindow` 2 s) or srv's servers
/// stopping on quit. [`app_exit`] waits for all of them.
static CLOSES: std::sync::LazyLock<tokio_util::task::TaskTracker> =
    std::sync::LazyLock::new(tokio_util::task::TaskTracker::new);

/// Run `fut` to completion as a tracked task, even if the caller is dropped
/// first; [`app_exit`] waits for it. `None` if it panicked.
pub(crate) async fn detached<T: Send + 'static>(fut: impl std::future::Future<Output = T> + Send + 'static) -> Option<T> {
    CLOSES.spawn(fut).await.ok()
}

/// Tear down every block in `block_ids` concurrently, under ONE deadline:
/// ten agents take one grace period, not ten. Each runs [`detached`].
pub async fn run_many(state: &AppState, block_ids: &[String], policy: Policy) -> Vec<TeardownReport> {
    let deadline = std::time::Instant::now() + blockcontroller::SHUTDOWN_GRACE;
    let teardowns = block_ids.iter().map(|id| {
        let (st, id) = (state.clone(), id.clone());
        async move {
            let report = detached({
                let id = id.clone();
                async move { run_one(&st, &id, policy, deadline).await }
            })
            .await;
            report.unwrap_or_else(|| {
                tracing::error!(block_id = %id, "agent_teardown: teardown panicked");
                TeardownReport { agent: id.clone(), block_id: id, ..Default::default() }
            })
        }
    });
    futures_util::future::join_all(teardowns).await
}

/// [`run_many`] for one block.
pub async fn run(state: &AppState, block_id: &str, policy: Policy) -> TeardownReport {
    run_many(state, std::slice::from_ref(&block_id.to_string()), policy)
        .await
        .pop()
        .unwrap_or_default()
}

/// [`Policy::stop_cli_only`]'s entry, for callers with no `AppState` (the
/// watchdog, the pane's `AgentStop` command): `controller.stop()` only.
/// Errors with `NOT_RUNNING` when the block has no controller.
pub fn stop_now(block_id: &str, graceful: bool) -> Result<(), String> {
    debug_assert!(matches!(Policy::stop_cli_only(graceful).cli, CliStop::StopOnly { .. }));
    let ctrl = blockcontroller::get_controller(block_id)
        .ok_or_else(|| format!("NOT_RUNNING: no controller for block {block_id}"))?;
    ctrl.stop(graceful, blockcontroller::STATUS_DONE)
}

/// Stop (`agent.stop`, `FleetBulkStop`, a deferred `Stop` action):
/// [`Policy::stop`]. `stop_background` is the caller's answer to "also stop
/// its background tasks?"; `None` takes the user's setting
/// ([`SETTING_STOP_KEEPS_BACKGROUND`]). Errors with `NOT_RUNNING` when the
/// block has no controller, as `agent.stop` always has.
pub async fn stop(state: &AppState, block_id: &str, graceful: bool, stop_background: Option<bool>) -> Result<TeardownReport, String> {
    if blockcontroller::get_controller(block_id).is_none() {
        return Err(format!("NOT_RUNNING: no controller for block {block_id}"));
    }
    let stop_background = stop_background.unwrap_or_else(|| !stop_keeps_background(state));
    let report = run(state, block_id, Policy::stop(graceful, stop_background)).await;
    match report.stop_error {
        Some(e) => Err(e),
        None => Ok(report),
    }
}

fn stop_keeps_background(state: &AppState) -> bool {
    state
        .config_watcher
        .get_settings()
        .extra
        .get(SETTING_STOP_KEEPS_BACKGROUND)
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Drop a block's controller and process tracker at once, with no graceful
/// phase: for a controller that isn't an agent to wind down (a PtyShell
/// drawer the user closed, a `/btw` throwaway, a spawn being rolled back) and
/// as the backstop after a close that already ran the teardown (idempotent;
/// finds nothing left).
pub fn discard(block_id: &str) {
    blockcontroller::delete_controller(block_id);
}

/// The replace policy's entry, for `resync_controller` (synchronous, below
/// the saga layer, and already holding the old controller): stop its own CLI
/// process only (`Policy::replace`).
pub fn replace_now(ctrl: &dyn blockcontroller::Controller) -> Result<(), String> {
    debug_assert!(matches!(Policy::replace().cli, CliStop::Replace));
    ctrl.stop_for_replace(blockcontroller::STATUS_DONE)
}

/// App exit (`main.rs`, after the servers stop): tear down every live agent
/// with `Policy::app_exit()`, concurrently, and wait for every close already
/// in flight, all under [`APP_EXIT_CAP`]. Then stop any `Shell()` session left
/// over (one whose agent is already gone, or whose teardown hit the cap).
/// Returns how many controllers it closed.
pub async fn app_exit(state: &AppState) -> usize {
    // Durable SSH panes detach rather than end their sessions: they live on
    // on their hosts for the next start to reattach (durable_ssh.rs).
    crate::backend::blockcontroller::durable_ssh::note_app_exiting();
    let all: Vec<String> = blockcontroller::get_all_controllers().into_keys().collect();
    // A drawer is closed by its parent's teardown (`close_sub_blocks`); listing
    // it here too would run two teardowns of one block at once.
    let ids = {
        let (st, ids) = (state.clone(), all.clone());
        tokio::task::spawn_blocking(move || without_sub_blocks(&st, ids)).await.unwrap_or(all)
    };
    let cap = APP_EXIT_CAP.saturating_sub(SHELL_SWEEP_GRACE);
    let all_closed = async {
        run_many(state, &ids, Policy::app_exit()).await;
        // And every close already in flight: its controller has left the
        // registry, so the list above doesn't have it (a window closed just
        // before the quit, whose host stopped waiting for the reply).
        CLOSES.close();
        CLOSES.wait().await;
    };
    if tokio::time::timeout(cap, all_closed).await.is_err() {
        tracing::warn!(agents = ids.len(), "app exit: teardown hit the cap; the launcher's backstop takes the rest");
    }
    // The kill tasks run taskkill/killpg asynchronously; give them a brief
    // grace to complete before srv exits. [reagent #1422 P2]
    let live = state.shell_sessions.stop_all();
    if live > 0 {
        tracing::info!(count = live, "app exit: stopping leftover persistent shells");
        tokio::time::sleep(SHELL_SWEEP_GRACE).await;
    }
    ids.len()
}

/// `ids` minus any that is a sub-block (PtyShell drawer) of another block in
/// it. Store reads: call off the async workers.
pub(crate) fn without_sub_blocks(state: &AppState, ids: Vec<String>) -> Vec<String> {
    let subs: std::collections::HashSet<String> = ids
        .iter()
        .filter_map(|id| state.mstore.get::<crate::backend::obj::Block>(id).ok().flatten())
        .flat_map(|b| b.subblockids.unwrap_or_default())
        .collect();
    ids.into_iter().filter(|id| !subs.contains(id)).collect()
}

async fn run_one(state: &AppState, block_id: &str, policy: Policy, deadline: std::time::Instant) -> TeardownReport {
    let started = std::time::Instant::now();
    // Identity and inventory while the block is still registered (step 2).
    let who = AgentIdentity::of(state, block_id);
    // Store reads (SQLite) off the async workers: a bulk close runs this
    // for every block at once.
    let before = {
        let (st, id) = (state.clone(), block_id.to_string());
        tokio::task::spawn_blocking(move || agent_resources::snapshot(&st, &id))
            .await
            .unwrap_or_default()
    };
    let mut report = TeardownReport {
        block_id: block_id.to_string(),
        agent: who.label(block_id),
        before: before.clone(),
        ..Default::default()
    };

    // Stop: the CLI first. If it fails the agent is still running, so its
    // claims and background tasks must stay too (ReAgent P1 on #4174).
    if let CliStop::StopOnly { graceful } = policy.cli {
        if let Err(e) = stop_now(block_id, graceful) {
            tracing::warn!(block_id = %block_id, error = %e, "agent_teardown: stop failed; nothing else touched");
            report.stop_error = Some(e);
            return report;
        }
    }

    // Step 3: non-process resources, so nothing new reaches a dying agent.
    if policy.release_claims {
        let reason = if matches!(policy.cli, CliStop::StopOnly { .. }) { "agent stopped" } else { "agent quit" };
        let (released, crons) = release_claims_and_list_crons(state, &who, reason).await;
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

    if policy.stop_background && !before.background_tasks.is_empty() {
        report.stopped_background = stop_background_tasks(&before).await;
    }

    match policy.cli {
        CliStop::StopOnly { .. } => {} // done first, above
        CliStop::Replace => {
            if let Some(ctrl) = blockcontroller::get_controller(block_id) {
                if let Err(e) = replace_now(ctrl.as_ref()) {
                    tracing::debug!(block_id = %block_id, error = %e, "agent_teardown: replace");
                }
            }
        }
        CliStop::Graceful => {
            // PtyShell drawers first (spec §6.6), so teardown doesn't depend on
            // the pane being mounted to send `deletesubblock`.
            close_sub_blocks(state, &before.sub_blocks, deadline).await;
            report.survivors = close_cli(state, block_id, &who, &before, deadline, started).await;
            if policy.stop_container {
                report.stopped_container = stop_container(state, block_id, &before, deadline).await;
            }
        }
    }
    report
}

/// Stop a container agent's container (spec §6.7), unless another live pane
/// runs the same agent in it. Docker's own SIGTERM → SIGKILL grace is bounded
/// by what's left of the teardown's deadline.
async fn stop_container(
    state: &AppState,
    block_id: &str,
    before: &AgentResources,
    deadline: std::time::Instant,
) -> Option<String> {
    let name = before.container.clone()?;
    let shared = {
        let (st, id, n) = (state.clone(), block_id.to_string(), name.clone());
        tokio::task::spawn_blocking(move || agent_resources::container_shared(&st, &id, &n)).await.unwrap_or(true)
    };
    if shared {
        tracing::info!(block_id = %block_id, container = %name, "agent_teardown: container still in use; kept");
        return None;
    }
    let cm = state.container_manager.get().await?;
    let grace = deadline.saturating_duration_since(std::time::Instant::now()).as_secs().max(1) as i64;
    match cm.stop(&name, grace).await {
        Ok(()) => {
            publish_shutdown(state, block_id, "container", format!("stopped container {name}"), serde_json::json!({}));
            Some(name)
        }
        Err(e) => {
            tracing::warn!(block_id = %block_id, container = %name, error = %e, "agent_teardown: container stop failed");
            None
        }
    }
}

/// Stop the agent's running `run_in_background` tasks, each with its process
/// tree, after re-checking each PID is still that task. Returns how many.
async fn stop_background_tasks(before: &AgentResources) -> usize {
    let tasks = before.background_tasks.clone();
    tokio::task::spawn_blocking(move || {
        let pids = agent_resources::live_background_pids(&tasks);
        for pid in &pids {
            tracing::info!(pid, "agent_teardown: stopping a background task");
            agentmux_common::process::kill_process_group(*pid);
        }
        pids.len()
    })
    .await
    .unwrap_or(0)
}

/// Close a parent's sub-blocks (PtyShell drawers) the way a pane closes,
/// concurrently and under the parent's deadline.
async fn close_sub_blocks(state: &AppState, sub_blocks: &[String], deadline: std::time::Instant) {
    if sub_blocks.is_empty() {
        return;
    }
    let subs = sub_blocks.iter().map(|id| Box::pin(run_one(state, id, Policy::close(), deadline)));
    futures_util::future::join_all(subs).await;
    // Their processes are gone. Nothing deletes a drawer's record through a
    // close saga's `finish_close`, so lift the respawn guard here: if the
    // parent's close then fails, its drawer can be reopened.
    for id in sub_blocks {
        blockcontroller::unmark_closing(id);
    }
}

/// The per-agent close (`SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md`
/// §4.2), then the verify step (agent teardown spec §6.2 step 8). Returns the
/// survivors.
async fn close_cli(
    state: &AppState,
    block_id: &str,
    who: &AgentIdentity,
    before: &AgentResources,
    deadline: std::time::Instant,
    started: std::time::Instant,
) -> Vec<agent_resources::ProcessEntry> {
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
    // 4b. What it started gets the same chance: SIGTERM to its whole tree,
    //     then a short wait, before the release below kills what's left.
    terminate_members(state, block_id, deadline).await;
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
    // 5b. Verify: everything in the snapshot is gone, else force it once
    //     more and report what's left.
    let survivors = verify_gone(&before.processes).await;
    for p in &survivors {
        publish_shutdown(
            state,
            block_id,
            "survivor",
            format!("still running: {} (pid {})", short_command(&p.command), p.pid),
            serde_json::json!({ "process": { "pid": p.pid, "name": short_command(&p.command), "outcome": "survived" } }),
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
        survivors = survivors.len(),
        "agent_shutdown"
    );
    survivors
}

/// The wait after SIGTERM to what an agent started: what's left of the
/// teardown's deadline, within these bounds.
const MEMBERS_GRACE_MIN: std::time::Duration = std::time::Duration::from_millis(500);
const MEMBERS_GRACE_MAX: std::time::Duration = std::time::Duration::from_secs(2);

/// Ask everything in the block's tree to exit and wait until it has, or the
/// grace passes. Returns at once where the tracker has no graceful signal
/// (Windows) or nothing is left.
async fn terminate_members(state: &AppState, block_id: &str, deadline: std::time::Instant) {
    let registry = state.process_tracker.clone();
    let id = block_id.to_string();
    let count = move || {
        let (r, id) = (registry.clone(), id.clone());
        tokio::task::spawn_blocking(move || r.member_count(&id))
    };
    if count().await.unwrap_or(0) == 0 {
        return;
    }
    let (r, id) = (state.process_tracker.clone(), block_id.to_string());
    if !tokio::task::spawn_blocking(move || r.terminate(&id)).await.unwrap_or(false) {
        return;
    }
    let grace = deadline.saturating_duration_since(std::time::Instant::now()).clamp(MEMBERS_GRACE_MIN, MEMBERS_GRACE_MAX);
    let until = std::time::Instant::now() + grace;
    while std::time::Instant::now() < until {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if count().await.unwrap_or(0) == 0 {
            return;
        }
    }
}

/// How long the verify step waits for the tracker's release to take effect,
/// before and after its one forced kill per survivor.
const VERIFY_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Spec §6.2 step 8: wait for every process in the snapshot to be gone. Any
/// still running (same PID and start time) gets one forced kill of its tree;
/// whatever is left after that is returned. Never kills a PID the OS has
/// reused: `survivors` re-checks the start time right before the kill.
async fn verify_gone(before: &[agent_resources::ProcessEntry]) -> Vec<agent_resources::ProcessEntry> {
    if before.is_empty() {
        return Vec::new();
    }
    let mut left = wait_gone(before.to_vec()).await;
    if left.is_empty() {
        return left;
    }
    for p in &left {
        tracing::warn!(pid = p.pid, command = %p.command, "agent_teardown: forcing a survivor");
        let (pid, started) = (p.pid, p.started_at_ms);
        let _ = tokio::task::spawn_blocking(move || agent_resources::force_kill_tree(pid, started)).await;
    }
    left = wait_gone(left).await;
    left
}

/// Poll until none of `procs` is running or [`VERIFY_WAIT`] passes; returns
/// the ones still running.
async fn wait_gone(mut procs: Vec<agent_resources::ProcessEntry>) -> Vec<agent_resources::ProcessEntry> {
    let until = std::time::Instant::now() + VERIFY_WAIT;
    loop {
        procs = tokio::task::spawn_blocking(move || agent_resources::survivors(&procs)).await.unwrap_or_default();
        if procs.is_empty() || std::time::Instant::now() >= until {
            return procs;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Release the agent's work claims, so the items go back to the pool now
/// rather than when the 120 s lease expires, and list the crons that target
/// it (kept: a cron is a deliberate schedule that resumes when the agent is
/// reopened). SQLite reads and writes, off the async workers.
async fn release_claims_and_list_crons(state: &AppState, who: &AgentIdentity, reason: &'static str) -> (usize, Vec<String>) {
    let (identity_store, shared_store) = (state.identity_store.clone(), state.shared_store.clone());
    let (a, u) = (who.agent_id.clone(), who.uid.clone());
    let result = tokio::task::spawn_blocking(move || {
        let now = agentmux_common::time::now_ms();
        let items = identity_store.work_queue_claimed_by(&a, u.as_deref().unwrap_or("")).unwrap_or_default();
        let released = agent_resources::claims_held_by(&items, &a, u.as_deref())
            .into_iter()
            .filter(|w| {
                matches!(
                    identity_store.work_queue_release(&w.id, &w.claimed_by, &w.claimed_by_uid, w.attempts, reason, now),
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
        assert_eq!(Policy::close(), Policy { cli: CliStop::Graceful, release_claims: false, stop_shell_sessions: true, stop_background: false, stop_container: true });
        assert_eq!(Policy::quit(), Policy { cli: CliStop::Graceful, release_claims: true, stop_shell_sessions: true, stop_background: false, stop_container: true });
        assert_eq!(Policy::stop(false, false).cli, CliStop::StopOnly { graceful: false });
        // Stop releases claims and keeps shells; background tasks as asked.
        assert!(Policy::stop(true, false).release_claims && !Policy::stop(true, false).stop_shell_sessions);
        assert!(Policy::stop(true, true).stop_background && !Policy::stop(true, false).stop_background);
        // Containers stop with a closing agent, never on Stop or a restart.
        assert!(Policy::close().stop_container && Policy::quit().stop_container && Policy::app_exit().stop_container);
        assert!(!Policy::stop(true, true).stop_container && !Policy::replace().stop_container);
        // The watchdog's stop is the CLI only.
        let w = Policy::stop_cli_only(true);
        assert!(!w.release_claims && !w.stop_shell_sessions && !w.stop_background);
        assert_eq!(Policy::replace(), Policy { cli: CliStop::Replace, release_claims: false, stop_shell_sessions: false, stop_background: false, stop_container: false });
        assert_eq!(Policy::app_exit(), Policy { cli: CliStop::Graceful, release_claims: false, stop_shell_sessions: true, stop_background: false, stop_container: true });
        // App exit covers the CLI's own grace and the shell sweep.
        assert!(APP_EXIT_CAP.saturating_sub(SHELL_SWEEP_GRACE) > blockcontroller::SHUTDOWN_GRACE);
    }

    /// The single path (spec §3 G1, §9.1): outside this module and the
    /// process machinery itself, nothing in srv stops an agent's processes.
    #[test]
    fn nothing_else_stops_an_agent() {
        // (file suffix, why it may) — every entry is a definition, the
        // machinery's own internals, or a phase-2 consumer named in the spec.
        const ALLOWED: &[(&str, &str)] = &[
            ("sagas/agent_teardown.rs", "the one path"),
            ("backend/blockcontroller/mod.rs", "defines release_block_processes and delete_controller"),
            ("backend/blockcontroller/persistent/", "the controller's own shutdown internals"),
            ("backend/blockcontroller/shell/", "the controller's own stop internals"),
            ("backend/process_tracker/", "the tracker itself"),
            ("backend/shell_node.rs", "ShellSessionRegistry's own stop/stop_all"),
            ("server/http_shell.rs", "the ShellStop tool: the agent stopping one shell on purpose"),
        ];
        const PATTERNS: &[&str] = &[
            "release_block_processes(",
            "shell_sessions.stop(",
            ".stop_all()",
            ".shutdown(deadline",
            "ctrl.stop(",
            ".stop_for_replace(",
            "delete_controller(",
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
                // Test modules in their own files (`tests.rs`, `tests/`).
                let is_test_file = rel.ends_with("tests.rs") || rel.contains("/tests/");
                if is_test_file || ALLOWED.iter().any(|(a, _)| rel.starts_with(a) || rel == *a) {
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
