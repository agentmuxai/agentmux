// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Self-quit — end ONE agent gracefully and close its own tab
// (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §3, §4.3; Phase 1).
//
// Order (§4.3):
// 1. Resolve who the agent is while it is still registered — shutdown
//    unregisters the block, and a later lookup falls back to the bare id.
// 2. Release its work claims, so the items go back to the pool now rather
//    than when the 120 s lease expires.
// 3. Stop the `Shell()` children it spawned. They are srv-owned, outside the
//    block's process tracker, so the Job Object teardown doesn't reach them.
// 4. `delete_block::run`: the graceful shutdown (`close_pane::shutdown_agents`),
//    a stack-aware layout prune that keeps sibling tabs, and the frontend
//    update for an already-loaded tree (§12.4).
// 5. Audit, success or failure.
//
// Crons that target the agent are kept (decided Q2: a cron is a deliberate
// schedule that resumes when the agent is reopened) and reported, so the
// user can pause or delete them if they meant "stop everything".

use serde::Serialize;

use crate::backend::storage::cron::CronJob;
use crate::backend::storage::work_queue::{work_state, WorkItem};
use crate::server::AppState;

/// Who asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitOrigin {
    /// The user typed `/quit` or `/exit` in the agent's composer.
    UserSlash,
    /// The agent's own `QuitSelf` tool, in a turn the user started (§6).
    AgentTool,
}

impl QuitOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            QuitOrigin::UserSlash => "user_slash",
            QuitOrigin::AgentTool => "agent_tool",
        }
    }
}

/// Why `QuitSelf` doesn't proceed at once (§6.3). Until Phase 2b's override
/// window exists, every one of these is a refusal (spec §9: the non-user path
/// ships with 2b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateRefusal {
    /// The controller can't say what started the turn (not a Claude
    /// persistent agent, or a start nothing labelled).
    Unknown,
    /// The turn was started by something other than the user.
    NotUserTurn,
    /// The user started it, but other input got in since.
    Tainted,
    /// The quoted instruction isn't in the message that started the turn.
    InstructionMismatch,
}

impl GateRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            GateRefusal::Unknown => "turn_origin_unknown",
            GateRefusal::NotUserTurn => "not_user_turn",
            GateRefusal::Tainted => "turn_tainted",
            GateRefusal::InstructionMismatch => "user_instruction_mismatch",
        }
    }
}

fn normalized(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// §6.3: the quote must appear in the user's message, whitespace- and
/// case-insensitively. An empty quote never matches.
pub fn quote_matches(user_text: &str, quote: &str) -> bool {
    let q = normalized(quote);
    !q.is_empty() && normalized(user_text).contains(&q)
}

/// The §6.3 gate: may `QuitSelf` proceed without asking the user?
pub fn gate(
    provenance: Option<&crate::backend::blockcontroller::health::TurnProvenance>,
    user_instruction: &str,
) -> Result<(), GateRefusal> {
    use crate::backend::blockcontroller::health::TurnOrigin;
    let p = provenance.ok_or(GateRefusal::Unknown)?;
    if p.origin != TurnOrigin::User {
        return Err(GateRefusal::NotUserTurn);
    }
    if p.tainted {
        return Err(GateRefusal::Tainted);
    }
    match p.user_text.as_deref() {
        Some(text) if quote_matches(text, user_instruction) => Ok(()),
        _ => Err(GateRefusal::InstructionMismatch),
    }
}

/// Blocks with a `QuitSelf` waiting for their turn to end (§4.2's
/// `mark_quitting`): a second call answers `already_quitting`.
static QUITTING: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

/// How long a scheduled quit waits for the turn to end (§4.2); after that
/// the graceful shutdown interrupts it.
pub const SELF_QUIT_TURN_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

pub fn is_quitting(block_id: &str) -> bool {
    QUITTING.lock().unwrap_or_else(|e| e.into_inner()).contains(block_id)
        || crate::backend::blockcontroller::is_closing(block_id)
}

/// §4.2: the tool call is part of the turn that asked for it, so quit when
/// that turn ends (or after `SELF_QUIT_TURN_GRACE`), not now. Returns false
/// when a quit is already scheduled or under way.
pub fn schedule_after_turn(state: &AppState, block_id: &str, detail: String) -> bool {
    {
        let mut q = QUITTING.lock().unwrap_or_else(|e| e.into_inner());
        if q.contains(block_id) || crate::backend::blockcontroller::is_closing(block_id) {
            return false;
        }
        q.insert(block_id.to_string());
    }
    let (state, block_id) = (state.clone(), block_id.to_string());
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + SELF_QUIT_TURN_GRACE;
        while tokio::time::Instant::now() < deadline
            && crate::backend::blockcontroller::get_controller(&block_id)
                .is_some_and(|c| c.get_runtime_status().turn_active)
        {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        if let Err(e) = run_with(&state, &block_id, QuitOrigin::AgentTool, Some(&detail)).await {
            tracing::warn!(block_id = %block_id, error = %e, "self-quit (agent tool) failed");
        }
        QUITTING.lock().unwrap_or_else(|e| e.into_inner()).remove(&block_id);
    });
    true
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct QuitSummary {
    /// `"quit"`, or `"already_quitting"` for a block whose close is already
    /// under way (a second `/quit`, or `/quit` racing the pane ×).
    pub status: &'static str,
    pub agent: String,
    pub released_claims: usize,
    pub stopped_shells: usize,
    /// Names of cron jobs still targeting the agent (kept, §4.3).
    pub crons_targeting: Vec<String>,
}

/// Does a claim held by (`claimed_by`, `claimed_by_uid`) belong to the agent?
/// Same holder rule as the store's release (`HOLDER_BY_UID`): by UID when
/// both sides have one, else by name.
pub fn holds_claim(claimed_by: &str, claimed_by_uid: &str, agent_id: &str, uid: Option<&str>) -> bool {
    match uid.filter(|u| !u.is_empty()) {
        Some(u) if !claimed_by_uid.is_empty() => claimed_by_uid == u,
        _ => !agent_id.is_empty() && claimed_by == agent_id,
    }
}

/// Does a cron job aimed at (`target`, `target_uid`) deliver to the agent?
/// By UID when the job has one, else by name (case-insensitive, like agent
/// addressing).
pub fn cron_targets(target: &str, target_uid: &str, agent_id: &str, uid: Option<&str>) -> bool {
    match uid.filter(|u| !u.is_empty()) {
        Some(u) if !target_uid.is_empty() => target_uid == u,
        _ => !agent_id.is_empty() && target.eq_ignore_ascii_case(agent_id),
    }
}

fn claims_held_by<'a>(items: &'a [WorkItem], agent_id: &str, uid: Option<&str>) -> Vec<&'a WorkItem> {
    items
        .iter()
        .filter(|w| w.state == work_state::CLAIMED && holds_claim(&w.claimed_by, &w.claimed_by_uid, agent_id, uid))
        .collect()
}

fn crons_targeting(jobs: &[CronJob], agent_id: &str, uid: Option<&str>) -> Vec<String> {
    jobs.iter()
        .filter(|j| cron_targets(&j.target, &j.target_uid, agent_id, uid))
        .map(|j| j.name.clone())
        .collect()
}

pub async fn run(state: &AppState, block_id: &str, origin: QuitOrigin) -> Result<QuitSummary, String> {
    run_with(state, block_id, origin, None).await
}

/// [`run`] with an audit detail (for `QuitSelf`: the reason and the user's
/// quoted instruction).
pub async fn run_with(state: &AppState, block_id: &str, origin: QuitOrigin, detail: Option<&str>) -> Result<QuitSummary, String> {
    if crate::backend::blockcontroller::is_closing(block_id) {
        return Ok(QuitSummary { status: "already_quitting", ..Default::default() });
    }
    let tab_id = {
        let s = state.srv_state.lock().await;
        match s.blocks.get(block_id) {
            Some(rec) => rec.tab_id.clone(),
            None => return Err(format!("QuitAgent: block not found: {block_id}")),
        }
    };

    // 1. Identity, before anything unregisters the block.
    let reg = state.reactive_handler.get_agent_by_block(block_id);
    let agent_id = reg.as_ref().map(|a| a.agent_id.clone()).unwrap_or_default();
    let uid = reg.as_ref().and_then(|a| a.uid.clone());
    let agent = if agent_id.is_empty() { block_id.to_string() } else { agent_id.clone() };

    // 2 + crons: SQLite reads and writes, off the async workers.
    let (identity_store, shared_store) = (state.identity_store.clone(), state.shared_store.clone());
    let (a, u) = (agent_id.clone(), uid.clone());
    let (released_claims, crons) = tokio::task::spawn_blocking(move || {
        let now = agentmux_common::time::now_ms();
        let items = identity_store.work_queue_claimed_by(&a, u.as_deref().unwrap_or("")).unwrap_or_default();
        let released = claims_held_by(&items, &a, u.as_deref())
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
            .map(|jobs| crons_targeting(&jobs, &a, u.as_deref()))
            .unwrap_or_default();
        (released, crons)
    })
    .await
    .map_err(|e| format!("QuitAgent: {e}"))?;
    if released_claims > 0 {
        crate::server::work_queue::publish_changed(state);
    }

    // 3. Srv-spawned shells this block owns.
    let stopped_shells = state
        .shell_sessions
        .list_active()
        .into_iter()
        .filter(|s| s.block_id == block_id)
        .filter(|s| state.shell_sessions.stop(&s.shell_id))
        .count();

    // 4. The graceful close of this tab only.
    let result = crate::sagas::delete_block::run(state, tab_id, block_id.to_string()).await;

    // 5. Audit.
    let request_id = uuid::Uuid::new_v4().to_string();
    state.reactive_handler.log_fleet_action_audit(
        None,
        &agent,
        block_id,
        "agent.quit",
        result.is_ok(),
        result.as_ref().err().map(String::as_str),
        &request_id,
        Some(&match detail {
            Some(d) => format!("{}: {d}", origin.as_str()),
            None => origin.as_str().to_string(),
        }),
    );
    tracing::info!(
        block_id,
        agent = %agent,
        origin = origin.as_str(),
        released_claims,
        stopped_shells,
        crons_targeting = crons.len(),
        ok = result.is_ok(),
        "self-quit"
    );
    result?;
    Ok(QuitSummary { status: "quit", agent, released_claims, stopped_shells, crons_targeting: crons })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;
    use agentmux_common::ipc::{Command, Event};

    #[test]
    fn claims_are_matched_by_uid_when_both_sides_have_one_else_by_name() {
        assert!(holds_claim("Lark", "uid-lark", "Lark", Some("uid-lark")));
        assert!(holds_claim("Lark", "", "Lark", Some("uid-lark")), "claimed before UIDs existed: by name");
        assert!(!holds_claim("Lark", "uid-other", "Lark", Some("uid-lark")), "a same-named other agent");
        assert!(!holds_claim("Korp", "uid-korp", "Lark", Some("uid-lark")));
        assert!(holds_claim("Lark", "uid-other", "Lark", None), "no UID of our own: by name");
        assert!(!holds_claim("", "", "", None), "an unknown agent releases nothing");
    }

    #[test]
    fn crons_targeting_the_agent_are_matched_by_uid_or_name() {
        assert!(cron_targets("Lark", "uid-lark", "Lark", Some("uid-lark")));
        assert!(cron_targets("lark", "", "Lark", Some("uid-lark")), "legacy job, name only, case-insensitive");
        assert!(!cron_targets("Lark", "uid-other", "Lark", Some("uid-lark")));
        assert!(!cron_targets("Korp", "uid-korp", "Lark", Some("uid-lark")));
    }

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
        events
    }

    async fn seed_two_tab_pane(state: &AppState) -> (String, String, String) {
        let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
            .await
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_id = dispatch_apply(state, Command::CreateTab { workspace_id: ws, name: "t".into() })
            .await
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        let mut blocks = Vec::new();
        for _ in 0..2 {
            blocks.push(
                dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null })
                    .await
                    .iter()
                    .find_map(|e| match e {
                        Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                        _ => None,
                    })
                    .unwrap(),
            );
        }
        let (a, b) = (blocks[0].clone(), blocks[1].clone());
        let leaf = agentmux_common::LayoutNode {
            id: "leaf".into(),
            data: Some(agentmux_common::LayoutNodeData {
                block_id: a.clone(),
                block_stack: vec![a.clone(), b.clone()],
                active_block_id: a.clone(),
                ..Default::default()
            }),
            ..Default::default()
        };
        dispatch_apply(
            state,
            Command::LayoutSetTree { tab_id: tab_id.clone(), new_tree: Some(leaf), correlation_id: String::new(), slices: None },
        )
        .await;
        (tab_id, a, b)
    }

    fn queued_deletes(state: &AppState, tab_id: &str) -> Vec<String> {
        let tab = state.mstore.must_get::<crate::backend::obj::Tab>(tab_id).unwrap();
        let layout = state.mstore.must_get::<crate::backend::obj::LayoutState>(&tab.layoutstate).unwrap();
        layout
            .pendingbackendactions
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.actiontype == "delete")
            .map(|a| a.blockid)
            .collect()
    }

    /// Spec §3.2 + §12.4: quitting one tab of a two-tab pane closes only that
    /// tab, keeps the pane and the sibling, and queues the frontend `delete`
    /// so an already-loaded frontend drops the tab instead of resurrecting it.
    #[tokio::test]
    async fn quitting_one_tab_keeps_the_pane_and_tells_the_frontend() {
        let state = test_state();
        let (tab_id, a, b) = seed_two_tab_pane(&state).await;

        let summary = run(&state, &a, QuitOrigin::UserSlash).await.expect("quit");
        assert_eq!(summary.status, "quit");
        assert_eq!((summary.released_claims, summary.stopped_shells), (0, 0));

        let s = state.srv_state.lock().await;
        assert!(!s.blocks.contains_key(&a), "the quitting tab is gone");
        assert!(s.blocks.contains_key(&b), "the sibling tab keeps running");
        assert!(s.tabs[&tab_id].rootnode.is_some(), "the pane survives");
        drop(s);
        assert_eq!(queued_deletes(&state, &tab_id), vec![a.clone()], "frontend told to drop the tab");

        assert!(run(&state, &a, QuitOrigin::UserSlash).await.is_err(), "already gone: not found");
    }

    #[test]
    fn the_quote_must_be_in_the_users_message_give_or_take_spacing_and_case() {
        assert!(quote_matches("Finish the PR, then quit.", "then quit"));
        assert!(quote_matches("finish the PR,\n  then   QUIT", "then quit"));
        assert!(!quote_matches("finish the PR", "then quit"), "not what the user said");
        assert!(!quote_matches("quit", ""), "an empty quote proves nothing");
    }

    #[test]
    fn the_gate_passes_only_an_untainted_user_turn_with_a_matching_quote() {
        use crate::backend::blockcontroller::health::{TurnOrigin, TurnProvenance};
        let p = |origin, tainted, text: Option<&str>| TurnProvenance { origin, tainted, user_text: text.map(Into::into) };
        let ok = p(TurnOrigin::User, false, Some("finish the PR then quit"));
        assert_eq!(gate(Some(&ok), "then quit"), Ok(()));
        assert_eq!(gate(None, "then quit"), Err(GateRefusal::Unknown));
        assert_eq!(gate(Some(&p(TurnOrigin::Automated, false, None)), "quit"), Err(GateRefusal::NotUserTurn));
        assert_eq!(gate(Some(&p(TurnOrigin::System, false, None)), "quit"), Err(GateRefusal::NotUserTurn));
        assert_eq!(gate(Some(&p(TurnOrigin::User, true, Some("quit"))), "quit"), Err(GateRefusal::Tainted));
        assert_eq!(gate(Some(&ok), "you may now exit"), Err(GateRefusal::InstructionMismatch));
    }

    #[tokio::test]
    async fn a_block_already_closing_reports_already_quitting_and_touches_nothing() {
        let state = test_state();
        let (_tab, a, _b) = seed_two_tab_pane(&state).await;
        crate::backend::blockcontroller::mark_closing(&a);
        let summary = run(&state, &a, QuitOrigin::UserSlash).await.unwrap();
        crate::backend::blockcontroller::unmark_closing(&a);
        assert_eq!(summary.status, "already_quitting");
        assert!(state.srv_state.lock().await.blocks.contains_key(&a), "left alone");
    }
}
