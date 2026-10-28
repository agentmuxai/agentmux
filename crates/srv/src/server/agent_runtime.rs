// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What state every created agent is in, with or without a pane: `agentruntime.list`, the
//! `agentruntime:changed:<agent id>` event that carries a change, and `agent.start`
//! (the always-available agents spec, §3.4 and §5).
//!
//! One source: the availability view (`availability_view`, which is what a message to the agent
//! asks), mapped onto the states a viewer sees. An agent with no live pane is `stopped`; its row
//! comes from its definition. A viewer (the Swarm) lists the rows once, then follows the event.
//!
//! A pane whose process ended keeps its `block_id` (found by the block's agent id) and reads `stopped`.
//!
//! Two things a viewer must not wait for until the always-available agents build lands: `starting`
//! is never produced (a controller just created, with no process yet, reads as `idle`), and no event
//! says an agent was deleted (`changed_rows` only forgets it), so a viewer relists on `agents:changed`.
//!
//! `desired` is `running` while a process is alive and `stopped` otherwise, until the agent record
//! keeps its own desired state (the always-available agents build's P1 PR C).

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::agent_availability::{Availability, WaitReason};
use crate::backend::obj::Block;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{COMMAND_AGENTRUNTIME_LIST, COMMAND_AGENT_START};
use crate::backend::storage::store::Store;

use super::availability_view::{availability_of_block, block_of_uid};
use super::AppState;

/// How often the runtime states are looked at for a change.
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// The state of one agent as a viewer sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeState {
    /// No process: never started, ended, or its pane closed.
    Stopped,
    /// A process is coming up and has not reported a state yet.
    Starting,
    Idle,
    Working,
    /// A turn is running and a question waits for the user.
    WaitingOnUser,
    /// Out of quota; `resets_at` when the provider said when it lifts.
    Limited,
    SignedOut,
    /// Its process ended with an error.
    Error,
}

/// Whether the agent should be running (spec rule 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum AgentDesiredState {
    Running,
    Stopped,
}

/// One agent's runtime state, with what a row needs to show it before it has a pane. The payload of
/// `agentruntime:changed:<agent id>` and a row of `agentruntime.list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentRuntime {
    /// The agent's UID, the id of `agentidentities:changed:<agent id>`.
    pub agent_id: String,
    pub state: AgentRuntimeState,
    /// Why, for a state that has a reason: `usage_limit` or `needs_login`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    /// When a limit lifts (unix ms), when it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub resets_at: Option<u64>,
    /// The block the agent runs in; absent for an agent that has no live pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub block_id: Option<String>,
    pub desired: AgentDesiredState,
    pub name: String,
    pub provider: String,
    pub icon: String,
    /// The colour its pane shows, once it has had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color: Option<String>,
}

/// `agent.start`.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandAgentStartData {
    pub agent_id: String,
}

/// What a start gave: the block the agent runs in (the parked block, for an agent never opened).
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AgentStartResult {
    pub agent_id: String,
    pub block_id: String,
}

/// The state, its reason and its reset, from what the availability view says.
pub fn state_of(av: &Availability) -> (AgentRuntimeState, Option<String>, Option<u64>) {
    match *av {
        Availability::NotRunning { exited_with_error: true } => (AgentRuntimeState::Error, None, None),
        Availability::NotRunning { exited_with_error: false } => (AgentRuntimeState::Stopped, None, None),
        // Nothing tracks the turns of a pane with no agent session in it: alive, and idle for a viewer.
        Availability::Idle { .. } | Availability::Unknown => (AgentRuntimeState::Idle, None, None),
        Availability::Running { waiting_on_user: true } => (AgentRuntimeState::WaitingOnUser, None, None),
        Availability::Running { waiting_on_user: false } => (AgentRuntimeState::Working, None, None),
        Availability::Limited { resets_at_ms, .. } => {
            (AgentRuntimeState::Limited, Some(WaitReason::UsageLimit.as_str().to_string()), resets_at_ms)
        }
        Availability::SignedOut { .. } => {
            (AgentRuntimeState::SignedOut, Some(WaitReason::NeedsLogin.as_str().to_string()), None)
        }
    }
}

/// A process is alive in every state but these two.
pub fn desired_of(state: AgentRuntimeState) -> AgentDesiredState {
    match state {
        AgentRuntimeState::Stopped | AgentRuntimeState::Error => AgentDesiredState::Stopped,
        _ => AgentDesiredState::Running,
    }
}

/// The runtime state of every created agent, as of `now_ms`. Blocking: reads the store.
pub(crate) fn list(state: &AppState, now_ms: u64) -> Vec<AgentRuntime> {
    let mstore = &state.mstore;
    // The definitions, not the local instance rows: an agent defined on another channel of this
    // host is in the shared registry before it has a row here, and `listagents` shows it too.
    // A template is not an agent.
    let defs = mstore.agent_def_list().unwrap_or_default();
    let panes = pane_blocks_by_agent(mstore);
    defs.into_iter()
        .filter(|d| d.is_seeded != 1 && d.user_hidden == 0)
        .map(|d| {
            // By the agent's UID, not its name: two agents can share a display name.
            let live = block_of_uid(state.reactive_handler, &d.id);
            // A pane whose process ended is no longer registered, but it is still the agent's
            // block: a viewer needs its id to open or stop it. Its state is `stopped`.
            let block_id = live.clone().or_else(|| panes.get(&d.id).cloned());
            let (runtime_state, detail, resets_at) = state_of(&availability_of_block(mstore, live.as_deref(), now_ms));
            AgentRuntime {
                color: block_id.as_deref().and_then(|b| color_of_block(mstore, b)),
                agent_id: d.id,
                state: runtime_state,
                detail,
                resets_at,
                block_id,
                desired: desired_of(runtime_state),
                name: d.name,
                provider: d.provider,
                icon: d.icon,
            }
        })
        .collect()
}

/// The block of each agent that has a pane, by agent id (one block when it has several: the first by
/// id). One pass over the blocks for the whole list.
fn pane_blocks_by_agent(mstore: &Store) -> HashMap<String, String> {
    let agent_blocks: Vec<String> = mstore
        .get_all::<Block>()
        .unwrap_or_default()
        .into_iter()
        .filter(|b| ["agentId", "agent:id"].iter().any(|k| !crate::backend::obj::meta_get_string(&b.meta, k, "").is_empty()))
        .map(|b| b.oid)
        .collect();
    let mut by_agent: HashMap<String, String> = HashMap::new();
    for pane in super::app_api::agent_io::open_agent_panes(mstore, &agent_blocks) {
        by_agent.entry(pane.agent_id).or_insert(pane.block_id);
    }
    by_agent
}

/// The colour the agent's pane shows (its border colour, `frame:activebordercolor`, set when it opens).
fn color_of_block(mstore: &Store, block_id: &str) -> Option<String> {
    let block = mstore.get::<Block>(block_id).ok()??;
    let color = crate::backend::obj::meta_get_string(&block.meta, "frame:activebordercolor", "");
    (!color.is_empty()).then_some(color)
}

/// The rows that differ from what was last sent, and what to remember now: an agent that is gone
/// is forgotten.
pub fn changed_rows(last: &mut HashMap<String, AgentRuntime>, now: Vec<AgentRuntime>) -> Vec<AgentRuntime> {
    let still: std::collections::HashSet<&str> = now.iter().map(|r| r.agent_id.as_str()).collect();
    last.retain(|id, _| still.contains(id.as_str()));
    let mut changed = Vec::new();
    for row in now {
        if last.get(&row.agent_id) != Some(&row) {
            last.insert(row.agent_id.clone(), row.clone());
            changed.push(row);
        }
    }
    changed
}

/// Look at the states on a short interval and publish each change to `agentruntime:changed:<id>`.
pub(crate) fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut last: HashMap<String, AgentRuntime> = HashMap::new();
        loop {
            let looked = state.clone();
            if let Ok(rows) = tokio::task::spawn_blocking(move || list(&looked, agentmux_common::time::now_ms_u64())).await {
                for row in changed_rows(&mut last, rows) {
                    state.broker.publish_event_for::<crate::backend::mps_events::AgentRuntimeChanged>(
                        &row.agent_id,
                        &row,
                        vec![],
                        0,
                    );
                }
            }
            tokio::time::sleep(LOOK_EVERY).await;
        }
    });
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // No argument: `Value` takes whatever the caller sends.
    let list_state = state.clone();
    engine.register_typed(COMMAND_AGENTRUNTIME_LIST, move |_data: serde_json::Value, _ctx| {
        let state = list_state.clone();
        async move {
            tokio::task::spawn_blocking(move || list(&state, agentmux_common::time::now_ms_u64()))
                .await
                .map_err(|e| format!("agentruntime.list: {e}"))
        }
    });
    engine.register_typed(COMMAND_AGENT_START, move |cmd: CommandAgentStartData, _ctx| async move {
        // The headless start comes with the agent's own desired state (the always-available agents
        // build, P1 PR C and P2b); until then an agent without a pane is started by opening it.
        Err::<AgentStartResult, String>(format!(
            "agent.start: not available yet: an agent without a pane is started by opening it ({})",
            cmd.agent_id
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, state: AgentRuntimeState) -> AgentRuntime {
        AgentRuntime {
            agent_id: id.into(),
            state,
            detail: None,
            resets_at: None,
            block_id: None,
            desired: desired_of(state),
            name: id.into(),
            provider: "claude".into(),
            icon: String::new(),
            color: None,
        }
    }

    #[test]
    fn every_availability_has_a_state_a_viewer_sees() {
        let s = |av| state_of(&av).0;
        assert_eq!(s(Availability::NotRunning { exited_with_error: false }), AgentRuntimeState::Stopped);
        assert_eq!(s(Availability::NotRunning { exited_with_error: true }), AgentRuntimeState::Error);
        assert_eq!(s(Availability::Idle { quiet_for_ms: None }), AgentRuntimeState::Idle);
        assert_eq!(s(Availability::Running { waiting_on_user: false }), AgentRuntimeState::Working);
        assert_eq!(s(Availability::Running { waiting_on_user: true }), AgentRuntimeState::WaitingOnUser);
        assert_eq!(s(Availability::SignedOut { since_ms: 1 }), AgentRuntimeState::SignedOut);
        assert_eq!(s(Availability::Unknown), AgentRuntimeState::Idle);
    }

    #[test]
    fn a_limit_says_why_and_when_it_lifts() {
        let (state, detail, resets) = state_of(&Availability::Limited { resets_at_ms: Some(9_000), since_ms: 7 });
        assert_eq!((state, detail.as_deref(), resets), (AgentRuntimeState::Limited, Some("usage_limit"), Some(9_000)));
        let (_, detail, resets) = state_of(&Availability::SignedOut { since_ms: 7 });
        assert_eq!((detail.as_deref(), resets), (Some("needs_login"), None));
    }

    #[test]
    fn a_process_is_wanted_running_in_every_state_but_stopped_and_error() {
        assert_eq!(desired_of(AgentRuntimeState::Stopped), AgentDesiredState::Stopped);
        assert_eq!(desired_of(AgentRuntimeState::Error), AgentDesiredState::Stopped);
        for state in [
            AgentRuntimeState::Starting,
            AgentRuntimeState::Idle,
            AgentRuntimeState::Working,
            AgentRuntimeState::WaitingOnUser,
            AgentRuntimeState::Limited,
            AgentRuntimeState::SignedOut,
        ] {
            assert_eq!(desired_of(state), AgentDesiredState::Running, "{state:?}");
        }
    }

    #[test]
    fn only_what_changed_is_sent_and_a_gone_agent_is_forgotten() {
        let mut last = HashMap::new();
        let first = changed_rows(&mut last, vec![row("a", AgentRuntimeState::Idle), row("b", AgentRuntimeState::Stopped)]);
        assert_eq!(first.len(), 2);
        assert!(changed_rows(&mut last, vec![row("a", AgentRuntimeState::Idle), row("b", AgentRuntimeState::Stopped)]).is_empty());
        let next = changed_rows(&mut last, vec![row("a", AgentRuntimeState::Working), row("b", AgentRuntimeState::Stopped)]);
        assert_eq!(
            next.iter().map(|r| (r.agent_id.as_str(), r.state)).collect::<Vec<_>>(),
            vec![("a", AgentRuntimeState::Working)]
        );
        // b is deleted, then created again: it is sent again, as new.
        assert!(changed_rows(&mut last, vec![row("a", AgentRuntimeState::Working)]).is_empty());
        let again = changed_rows(&mut last, vec![row("a", AgentRuntimeState::Working), row("b", AgentRuntimeState::Stopped)]);
        assert_eq!(again.len(), 1);
    }

    #[test]
    fn the_wire_names_are_snake_case_and_leave_out_what_is_absent() {
        let mut r = row("a", AgentRuntimeState::WaitingOnUser);
        r.block_id = Some("blk".into());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["state"], "waiting_on_user");
        assert_eq!(json["desired"], "running");
        assert_eq!(json["block_id"], "blk");
        assert!(json.get("detail").is_none() && json.get("resets_at").is_none() && json.get("color").is_none());
    }

    /// A created agent (a definition that is not a template) with no pane is listed from its definition, as stopped and not wanted running;
    /// no block, no colour.
    #[tokio::test]
    async fn an_agent_with_no_pane_is_listed_as_stopped_from_its_definition() {
        let state = crate::server::tests::test_state();
        let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": "def-rt-1", "slug": "rt-one", "name": "Rt One", "icon": "robot", "provider": "claude",
            "description": "", "created_at": 1,
        }))
        .unwrap();
        state.mstore.agent_def_insert(&mut def).unwrap();
        let rows = list(&state, 10_000);
        let one = rows.iter().find(|r| r.agent_id == "def-rt-1").expect("the agent is listed");
        assert_eq!((one.state, one.desired), (AgentRuntimeState::Stopped, AgentDesiredState::Stopped));
        assert_eq!((one.name.as_str(), one.provider.as_str(), one.icon.as_str()), ("Rt One", "claude", "robot"));
        assert!(one.block_id.is_none() && one.color.is_none() && one.detail.is_none());
    }
}
