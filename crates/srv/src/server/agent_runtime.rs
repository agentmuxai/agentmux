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
//! An agent wanted running with no process reads `idle`, not `stopped`: a persistent agent with no
//! session on file starts on its first message, and an agent that has only ended its process is
//! started by the next one. The failure states (limited, signed out, waiting) keep their place.
//!
//! An agent between turns whose Browser request is still open reads `waiting_on_user` (`browser_request`,
//! with the pane the request waits in): its call answered "waiting" and it ended its turn, so nothing else
//! says the person is asked. Only what a viewer sees: the availability a message asks is unchanged, so a
//! message still starts a turn. The same fact is kept in the block's `term:awaiting_browser` for the rows
//! that read block meta (the Swarm's).
//!
//! `desired` is the agent's own desired state when its block records one (`agent:desired`, set by an
//! open, a start and a stop), and otherwise `running` while a process is alive and `stopped` if not.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::agent_availability::{Availability, WaitReason};
use crate::backend::agent_status::{AgentStatus, Wanted};
use crate::backend::obj::Block;
use crate::server::app_api::agent_attach::{self, DESIRED_RUNNING, DESIRED_STOPPED};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{COMMAND_AGENTRUNTIME_LIST, COMMAND_AGENT_START};
use crate::backend::storage::store::Store;

use super::availability_view::{block_of_uid, status_of};
use super::AppState;

/// How often the runtime states are looked at for a change.
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

pub use crate::backend::agent_runtime_state::{AgentDesiredState, AgentRuntime, AgentRuntimeState};

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

/// Why an agent wanted running with no process reads `idle` (`state_of`'s reasons are codes too).
pub const STARTS_ON_FIRST_MESSAGE: &str = "starts_on_first_message";

/// What a viewer sees of an agent that the availability view calls stopped but its block says should
/// run and a controller holds: it is waiting for a message to start it, which is `idle`. With no
/// controller (the server restarted and nothing resumed it yet) it stays `stopped`: the Swarm lists
/// only controller-backed agents as live, so an idle row there would be listed nowhere, with no Start.
pub fn wanted_running_is_idle(
    state: AgentRuntimeState,
    detail: Option<String>,
    desired: AgentDesiredState,
    has_controller: bool,
) -> (AgentRuntimeState, Option<String>) {
    if state == AgentRuntimeState::Stopped && desired == AgentDesiredState::Running && has_controller {
        (AgentRuntimeState::Idle, Some(STARTS_ON_FIRST_MESSAGE.to_string()))
    } else {
        (state, detail)
    }
}

/// Why an agent between turns reads `waiting_on_user`: a Browser request of its is still open.
pub const BROWSER_REQUEST: &str = "browser_request";

/// Block meta, true while the agent's row waits on a Browser request (the Swarm row reads block meta).
pub const META_AWAITING_BROWSER: &str = "term:awaiting_browser";

/// An idle agent with an open Browser request is waiting for the person. Only an idle one: an agent that
/// works is working (a message started another turn), and one that cannot work says that first.
pub fn waiting_on_browser_request(
    state: AgentRuntimeState,
    detail: Option<String>,
    request_open: bool,
) -> (AgentRuntimeState, Option<String>) {
    if request_open && state == AgentRuntimeState::Idle {
        (AgentRuntimeState::WaitingOnUser, Some(BROWSER_REQUEST.to_string()))
    } else {
        (state, detail)
    }
}

/// The pane where the open Browser request of the agent bound under `names` waits. A request carries the
/// name its call used, which is one of the names the agent's block is bound under.
fn request_pane(names: &[String]) -> Option<String> {
    names.iter().find_map(|n| super::browser_waiting::pending_for(n))
}

fn waits_on_browser_request(row: &AgentRuntime) -> bool {
    row.state == AgentRuntimeState::WaitingOnUser && row.detail.as_deref() == Some(BROWSER_REQUEST)
}

/// What to write to the block's [`META_AWAITING_BROWSER`] so it says what the row says, if it does not.
fn browser_flag_fix(row: &AgentRuntime, flag_now: bool) -> Option<bool> {
    let wanted = waits_on_browser_request(row);
    (wanted != flag_now).then_some(wanted)
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

/// What the agent's block says it should be doing, when it says: a stop sets `stopped`, an open or a
/// start `running`. An agent whose block records nothing is wanted as its state says.
pub fn desired_from(recorded: Option<&str>, state: AgentRuntimeState) -> AgentDesiredState {
    match recorded {
        Some(DESIRED_RUNNING) => AgentDesiredState::Running,
        Some(DESIRED_STOPPED) => AgentDesiredState::Stopped,
        _ => desired_of(state),
    }
}

/// What a viewer sees of an agent from its status: the state, why, when a limit lifts and what is
/// wanted. The one place the availability, the wanted state and the controller meet (`state_of`,
/// `desired_from` and `wanted_running_is_idle` are its steps).
pub fn runtime_of(status: &AgentStatus) -> (AgentRuntimeState, Option<String>, Option<u64>, AgentDesiredState) {
    let (state, detail, resets_at) = state_of(&status.availability);
    let recorded = match status.wanted {
        Wanted::Running => Some(DESIRED_RUNNING),
        Wanted::Stopped => Some(DESIRED_STOPPED),
        Wanted::Unset => None,
    };
    let desired = desired_from(recorded, state);
    let (state, detail) = wanted_running_is_idle(state, detail, desired, status.controller);
    (state, detail, resets_at, desired)
}

/// Why a start would be refused, for an agent that is stopped and wanted running: its block says running,
/// or a message waits for it. `why` is asked only then, because it reads every live agent.
fn blocked_start(state: AgentRuntimeState, desired: AgentDesiredState, message_waits: bool, why: impl FnOnce() -> Option<String>) -> Option<String> {
    (state == AgentRuntimeState::Stopped && (desired == AgentDesiredState::Running || message_waits)).then(why).flatten()
}

/// The runtime state of every created agent, as of `now_ms`. Blocking: reads the store.
pub(crate) fn list(state: &AppState, now_ms: u64) -> Vec<AgentRuntime> {
    let mstore = &state.mstore;
    // The definitions, not the local instance rows: an agent defined on another channel of this
    // host is in the shared registry before it has a row here, and `listagents` shows it too.
    // A template is not an agent.
    let defs = mstore.agent_def_list().unwrap_or_default();
    let panes = pane_blocks_by_agent(mstore);
    let net = crate::connectivity::status();
    let waiting_for: std::collections::HashSet<String> = mstore.jekt_held_targets().unwrap_or_default().into_iter().collect();
    let requests_open = super::browser_waiting::any_open();
    defs.into_iter()
        .filter(|d| d.is_seeded != 1 && d.user_hidden == 0)
        .map(|d| {
            // By the agent's UID, not its name: two agents can share a display name.
            let live = block_of_uid(state.reactive_handler, &d.id);
            // A pane whose process ended is no longer registered, and a headless agent's block is in no
            // tab, but each is still the agent's block: a viewer needs its id to open or stop it, and
            // it holds the desired state. The state is `stopped`.
            let block_id = live.clone().or_else(|| panes.get(&d.id).cloned());
            let block = block_id.as_deref().and_then(|b| mstore.get::<Block>(b).ok().flatten());
            let color = block.as_ref().and_then(block_color);
            let (runtime_state, detail, resets_at, desired) = runtime_of(&status_of(mstore, live.as_deref(), block.as_ref(), &net, now_ms));
            let blocked = blocked_start(runtime_state, desired, waiting_for.contains(&d.id), || super::agent_lifecycle::start_blocked(state, &d.id));
            let waiting_in = (requests_open && runtime_state == AgentRuntimeState::Idle)
                .then(|| live.as_deref().and_then(|b| request_pane(&state.reactive_handler.names_for_block(b))))
                .flatten();
            let (runtime_state, detail) = waiting_on_browser_request(runtime_state, detail, waiting_in.is_some());
            AgentRuntime {
                blocked,
                color,
                agent_id: d.id,
                state: runtime_state,
                detail,
                resets_at,
                block_id,
                waiting_in,
                desired,
                name: d.name,
                provider: d.provider,
                icon: d.icon,
            }
        })
        .collect()
}

/// The agent a block belongs to, by the id its meta carries.
fn agent_of_block(block: &Block) -> Option<String> {
    if crate::backend::obj::is_history_reader(&block.meta) {
        return None; // the reader of its transcript, not its pane
    }
    ["agentId", "agent:id"]
        .iter()
        .map(|k| crate::backend::obj::meta_get_string(&block.meta, k, ""))
        .find(|v| !v.is_empty())
}

/// The block of each agent that has one, by agent id: the one in a pane (the first by id when it has
/// several), else the one parked in no tab, which holds the agent's desired state while it has no
/// process. One pass over the blocks for the whole list.
pub(super) fn pane_blocks_by_agent(mstore: &Store) -> HashMap<String, String> {
    let blocks = mstore.get_all::<Block>().unwrap_or_default();
    let agent_blocks: Vec<String> = blocks.iter().filter(|b| agent_of_block(b).is_some()).map(|b| b.oid.clone()).collect();
    let mut by_agent: HashMap<String, String> = HashMap::new();
    for pane in super::app_api::agent_io::open_agent_panes(mstore, &agent_blocks) {
        by_agent.entry(pane.agent_id).or_insert(pane.block_id);
    }
    for block in &blocks {
        let parked = crate::backend::obj::meta_get_bool(&block.meta, crate::persist_subscriber::parked::PARKED_META_KEY, false);
        if let (true, Some(agent)) = (parked, agent_of_block(block)) {
            by_agent.entry(agent).or_insert_with(|| block.oid.clone());
        }
    }
    by_agent
}

/// The colour the agent's pane shows (its border colour, `frame:activebordercolor`, set when it opens).
fn block_color(block: &Block) -> Option<String> {
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

/// Whether `row` just began waiting on the user: it is waiting now and was not at the last look. An
/// agent with no block has nothing to notify about, and a Browser request was announced by its own
/// banner, on the Browser pane, under the person's setting for it.
fn begins_waiting(before: Option<AgentRuntimeState>, row: &AgentRuntime) -> bool {
    row.state == AgentRuntimeState::WaitingOnUser
        && row.block_id.is_some()
        && before != Some(AgentRuntimeState::WaitingOnUser)
        && !waits_on_browser_request(row)
}

/// The block an agent was waiting in, when it just stopped waiting (answered, cancelled or ended): the
/// block of the last look, which the row may no longer name.
fn ends_waiting(before: Option<&(AgentRuntimeState, Option<String>)>, row: &AgentRuntime) -> Option<String> {
    match before {
        Some((AgentRuntimeState::WaitingOnUser, Some(block_id))) if row.state != AgentRuntimeState::WaitingOnUser => Some(block_id.clone()),
        _ => None,
    }
}

/// Take down the notice [`notify_waiting`] raised, so a wait that ended before it showed does not show
/// a stale question, and the next wait on the block is a new notice and not coalesced into the old.
fn resolve_waiting(state: &AppState, block_id: &str) {
    if let Some(router) = crate::backend::notify::router::get(&state.broker) {
        router.resolve_nonblocking(block_id, crate::backend::notify::policy::Family::Input);
    }
}

/// "Waiting on you" as an OS notification, for an agent no pane need show (a headless or parked one):
/// the router coalesces it with the question's own report for the same block, so it adds nothing
/// when the question already notified, and covers a wait that had no such report (an App Server
/// approval).
fn notify_waiting(state: &AppState, row: &AgentRuntime) {
    let (Some(block_id), Some(router)) = (row.block_id.as_deref(), crate::backend::notify::router::get(&state.broker)) else {
        return;
    };
    router.input_waiting_nonblocking(block_id, None, 0);
}

/// The blocks of `rows` whose [`META_AWAITING_BROWSER`] does not say what the row says, with what it
/// should say. Blocking: reads the store.
fn browser_flag_fixes(mstore: &Store, rows: &[AgentRuntime]) -> Vec<(String, bool)> {
    rows.iter()
        .filter_map(|row| {
            let block_id = row.block_id.as_deref()?;
            let block = mstore.get::<Block>(block_id).ok().flatten()?;
            let flag_now = crate::backend::obj::meta_get_bool(&block.meta, META_AWAITING_BROWSER, false);
            browser_flag_fix(row, flag_now).map(|on| (block_id.to_string(), on))
        })
        .collect()
}

/// Keep each changed row's block meta saying whether it waits on a Browser request. Every row is a change
/// at the first look after a start, so a flag a stopped srv left behind is taken down then.
async fn mirror_browser_wait(state: &AppState, changed: &[AgentRuntime]) {
    let (looked, rows) = (state.clone(), changed.to_vec());
    let Ok(fixes) = tokio::task::spawn_blocking(move || browser_flag_fixes(&looked.mstore, &rows)).await else { return };
    for (block_id, on) in fixes {
        let flag = if on { serde_json::Value::Bool(true) } else { serde_json::Value::Null };
        let meta = serde_json::json!({ META_AWAITING_BROWSER: flag });
        if let Err(e) = super::agent_status_handlers::write_block_meta(state, &block_id, meta).await {
            tracing::warn!(block_id = %block_id, error = %e, "[agent-runtime] couldn't write the block's Browser-request flag; its Swarm row may be out of date");
        }
    }
}

/// Look at the states on a short interval and publish each change to `agentruntime:changed:<id>`.
pub(crate) fn spawn(state: AppState) {
    super::registry_reconcile::spawn(state.clone()); // a running agent missing from the registry is put back
    tokio::spawn(async move {
        let mut last: HashMap<String, AgentRuntime> = HashMap::new();
        loop {
            let looked = state.clone();
            if let Ok(rows) = tokio::task::spawn_blocking(move || list(&looked, agentmux_common::time::now_ms_u64())).await {
                let before: HashMap<String, (AgentRuntimeState, Option<String>)> =
                    last.iter().map(|(id, r)| (id.clone(), (r.state, r.block_id.clone()))).collect();
                let changed = changed_rows(&mut last, rows);
                mirror_browser_wait(&state, &changed).await;
                for row in changed {
                    let was = before.get(&row.agent_id);
                    if begins_waiting(was.map(|w| w.0), &row) {
                        notify_waiting(&state, &row);
                    }
                    if let Some(block_id) = ends_waiting(was, &row) {
                        resolve_waiting(&state, &block_id);
                    }
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
    // Not behind `agents:always_available`: a start is an explicit action and changes nothing for
    // anyone who does not call it; only closing a pane parking its block depends on the setting.
    let start_state = state.clone();
    engine.register_typed(COMMAND_AGENT_START, move |cmd: CommandAgentStartData, _ctx| {
        let state = start_state.clone();
        async move {
            let block_id = agent_attach::start_agent(&state, &cmd.agent_id)
                .await
                .map_err(|e| format!("agent.start: {e}"))?;
            Ok::<AgentStartResult, String>(AgentStartResult { agent_id: cmd.agent_id, block_id })
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_agent_something_wants_running_says_why_it_cannot_start_and_no_other_is_asked() {
        use AgentDesiredState::{Running, Stopped};
        use AgentRuntimeState::{Idle, Stopped as Down};
        let why = || Some("capacity: memory commit is at 89%".to_string());
        assert_eq!(blocked_start(Down, Stopped, true, why).as_deref(), Some("capacity: memory commit is at 89%"), "a message waits for it");
        assert_eq!(blocked_start(Down, Running, false, why).as_deref(), Some("capacity: memory commit is at 89%"), "its block says running");
        let asked = std::cell::Cell::new(0);
        let counted = || {
            asked.set(asked.get() + 1);
            Some("x".to_string())
        };
        assert_eq!(blocked_start(Down, Stopped, false, counted), None, "stopped on purpose, nothing waits");
        assert_eq!(blocked_start(Idle, Running, true, counted), None, "it runs");
        assert_eq!(asked.get(), 0, "the check reads every live agent: only a wanted, stopped one asks");
        assert_eq!(blocked_start(Down, Running, false, || None), None, "nothing refuses the start");
    }

    fn row(id: &str, state: AgentRuntimeState) -> AgentRuntime {
        AgentRuntime {
            agent_id: id.into(),
            state,
            detail: None,
            resets_at: None,
            block_id: None,
            waiting_in: None,
            desired: desired_of(state),
            name: id.into(),
            provider: "claude".into(),
            icon: String::new(),
            color: None,
            blocked: None,
        }
    }

    #[test]
    fn an_idle_agent_with_an_open_browser_request_waits_for_the_person() {
        use AgentRuntimeState as S;
        assert_eq!(waiting_on_browser_request(S::Idle, None, true), (S::WaitingOnUser, Some(BROWSER_REQUEST.to_string())));
        assert_eq!(
            waiting_on_browser_request(S::Idle, Some(STARTS_ON_FIRST_MESSAGE.to_string()), true),
            (S::WaitingOnUser, Some(BROWSER_REQUEST.to_string())),
            "an agent whose process ended is still asked for"
        );
        assert_eq!(waiting_on_browser_request(S::Idle, None, false), (S::Idle, None), "no request, no change");
        // A message started another turn: it works. And an agent that cannot work says that first.
        for state in [S::Working, S::WaitingOnUser, S::Limited, S::SignedOut, S::Stopped, S::Error, S::Starting] {
            let why = Some("usage_limit".to_string());
            assert_eq!(waiting_on_browser_request(state, why.clone(), true), (state, why), "{state:?} keeps its place");
        }
    }

    #[test]
    fn the_request_is_found_under_any_name_the_agents_block_is_bound_under() {
        let id = uuid::Uuid::new_v4().to_string();
        crate::server::browser_waiting::open(&id, "rt-asker", "pane-rt-asker", "approval", "Click Send", std::time::Duration::from_secs(600));
        assert!(crate::server::browser_waiting::any_open());
        // The block is bound under its display name and its slug; the call carried the slug, in another case.
        let names = ["Rt Asker".to_string(), "RT-ASKER".to_string()];
        assert_eq!(request_pane(&names).as_deref(), Some("pane-rt-asker"));
        assert_eq!(request_pane(&["someone-else".to_string()]), None);
        assert!(crate::server::browser_waiting::finish(&id, crate::server::browser_waiting::State::Declined));
        assert_eq!(request_pane(&names), None, "an answered request is over");
    }

    #[test]
    fn a_browser_request_is_not_announced_a_second_time_and_its_flag_follows_the_row() {
        use AgentRuntimeState as S;
        let mut asks = row("a", S::WaitingOnUser);
        asks.block_id = Some("block-a".into());
        assert!(begins_waiting(Some(S::Working), &asks), "a question in a turn is announced");
        asks.detail = Some(BROWSER_REQUEST.into());
        assert!(!begins_waiting(Some(S::Idle), &asks), "its banner on the Browser pane announced it");
        assert_eq!(ends_waiting(Some(&(S::WaitingOnUser, Some("block-a".into()))), &row("a", S::Idle)).as_deref(), Some("block-a"));
        assert_eq!(browser_flag_fix(&asks, false), Some(true));
        assert_eq!(browser_flag_fix(&asks, true), None, "already says so: nothing is written");
        assert_eq!(browser_flag_fix(&row("a", S::Idle), true), Some(false), "answered, or left by a stopped srv");
        assert_eq!(browser_flag_fix(&row("a", S::WaitingOnUser), true), Some(false), "a question in a turn is not a Browser request");
        assert_eq!(browser_flag_fix(&row("a", S::Working), false), None);
    }

    #[test]
    fn the_history_reader_is_not_the_agents_pane() {
        let mut meta = crate::backend::obj::MetaMapType::new();
        meta.insert("agentId".to_string(), serde_json::json!("ann"));
        let mut block = Block { meta, ..Default::default() };
        assert_eq!(agent_of_block(&block).as_deref(), Some("ann"));
        block.meta.insert(crate::backend::obj::HISTORY_READER_META_KEY.to_string(), serde_json::json!("ann"));
        assert_eq!(agent_of_block(&block), None, "a reader carries the id but is not the pane");
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
    fn the_blocks_own_desired_state_wins_over_the_state_it_is_in() {
        use AgentRuntimeState as S;
        // Stopped by the user while the process is still ending, or left wanted after a crash.
        assert_eq!(desired_from(Some("stopped"), S::Working), AgentDesiredState::Stopped);
        assert_eq!(desired_from(Some("running"), S::Stopped), AgentDesiredState::Running);
        assert_eq!(desired_from(Some("running"), S::Error), AgentDesiredState::Running);
        // Nothing recorded, or something this build does not know: as the state says.
        assert_eq!(desired_from(None, S::Idle), AgentDesiredState::Running);
        assert_eq!(desired_from(None, S::Stopped), AgentDesiredState::Stopped);
        assert_eq!(desired_from(Some("later"), S::Stopped), AgentDesiredState::Stopped);
    }

    /// The whole path: a registered agent between turns, a request opened under its slug, the row, what
    /// the block's flag should say, and both back once the request is answered.
    #[tokio::test]
    async fn an_open_browser_request_shows_in_the_agents_row_and_its_blocks_flag_until_it_is_answered() {
        let state = crate::server::tests::test_state();
        let uid = uuid::Uuid::new_v4().to_string();
        let slug = format!("rt-web-{}", &uid[..8]);
        let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": uid, "slug": slug, "name": "Rt Web", "icon": "robot", "provider": "claude",
            "description": "", "created_at": 1,
        }))
        .unwrap();
        state.mstore.agent_def_insert(&mut def).unwrap();
        let mut meta = crate::backend::obj::MetaMapType::new();
        meta.insert("agentId".to_string(), serde_json::json!(uid));
        let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), meta, ..Default::default() };
        state.mstore.insert(&mut block).unwrap();
        state.reactive_handler.register_agent_full(&slug, &block.oid, None, 1, Some(&slug), Some(&uid), "test.runtime").unwrap();
        let row_now = |state: &AppState| list(state, 10_000).into_iter().find(|r| r.agent_id == uid).expect("the agent is listed");
        // What `mirror_browser_wait` writes through the object service, here straight to the store.
        let write = |state: &AppState, fixes: Vec<(String, bool)>| {
            for (block_id, on) in fixes {
                let mut b = state.mstore.get::<Block>(&block_id).unwrap().unwrap();
                if on {
                    b.meta.insert(META_AWAITING_BROWSER.to_string(), serde_json::json!(true));
                } else {
                    b.meta.remove(META_AWAITING_BROWSER);
                }
                state.mstore.update(&mut b).unwrap();
            }
        };

        let before = row_now(&state);
        assert_eq!((before.state, before.detail.as_deref(), before.waiting_in.as_deref()), (AgentRuntimeState::Idle, None, None));

        // Its Browser call answered "waiting" and it ended its turn. The call carried its slug.
        let request = uuid::Uuid::new_v4().to_string();
        let pane = format!("web-{request}");
        crate::server::browser_waiting::open(&request, &slug.to_uppercase(), &pane, "approval", "Click Send", std::time::Duration::from_secs(600));
        let asks = row_now(&state);
        assert_eq!(asks.state, AgentRuntimeState::WaitingOnUser);
        assert_eq!((asks.detail.as_deref(), asks.waiting_in.as_deref()), (Some(BROWSER_REQUEST), Some(pane.as_str())));
        assert_eq!(asks.desired, AgentDesiredState::Running);
        let fixes = browser_flag_fixes(&state.mstore, std::slice::from_ref(&asks));
        assert_eq!(fixes, vec![(block.oid.clone(), true)], "the Swarm row reads the block's meta");
        write(&state, fixes);
        assert_eq!(browser_flag_fixes(&state.mstore, std::slice::from_ref(&asks)), vec![], "it says so now");

        // Answered: idle again, and the flag comes down with the row.
        assert!(crate::server::browser_waiting::finish(&request, crate::server::browser_waiting::State::Declined));
        let after = row_now(&state);
        assert_eq!((after.state, after.detail.as_deref(), after.waiting_in.as_deref()), (AgentRuntimeState::Idle, None, None));
        let fixes = browser_flag_fixes(&state.mstore, std::slice::from_ref(&after));
        assert_eq!(fixes, vec![(block.oid.clone(), false)]);
        write(&state, fixes);
        assert_eq!(browser_flag_fixes(&state.mstore, std::slice::from_ref(&after)), vec![]);
    }

    /// A headless agent whose process ended without a stop: its parked block is in no tab, still records
    /// that it should run, and is the block a viewer opens.
    #[tokio::test]
    async fn a_parked_block_with_no_controller_keeps_its_agent_wanted_running_but_stopped() {
        let state = crate::server::tests::test_state();
        let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
            "id": "def-rt-2", "slug": "rt-two", "name": "Rt Two", "icon": "robot", "provider": "claude",
            "description": "", "created_at": 1,
        }))
        .unwrap();
        state.mstore.agent_def_insert(&mut def).unwrap();
        let mut meta = crate::backend::obj::MetaMapType::new();
        meta.insert("agentId".to_string(), serde_json::json!("def-rt-2"));
        meta.insert(crate::persist_subscriber::parked::PARKED_META_KEY.to_string(), serde_json::json!(true));
        meta.insert(agent_attach::META_DESIRED.to_string(), serde_json::json!("running"));
        let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), meta, ..Default::default() };
        state.mstore.insert(&mut block).unwrap();
        let rows = list(&state, 10_000);
        let one = rows.iter().find(|r| r.agent_id == "def-rt-2").expect("the agent is listed");
        // No controller holds it (a restart): stopped, still wanted running, so the Swarm offers Start.
        assert_eq!((one.state, one.desired), (AgentRuntimeState::Stopped, AgentDesiredState::Running));
        assert!(one.detail.is_none());
        assert_eq!(one.block_id.as_deref(), Some(block.oid.as_str()));
    }

    #[test]
    fn an_agent_wanted_running_with_no_process_is_idle_and_says_it_starts_on_its_first_message() {
        use AgentRuntimeState as S;
        let (state, detail) = wanted_running_is_idle(S::Stopped, None, AgentDesiredState::Running, true);
        assert_eq!((state, detail.as_deref()), (S::Idle, Some("starts_on_first_message")));
        // Stopped on purpose, or with nothing said (a block from before the feature): as it was.
        assert_eq!(wanted_running_is_idle(S::Stopped, None, AgentDesiredState::Stopped, true), (S::Stopped, None));
        // Wanted running with no controller (after a restart): still stopped, so the Swarm lists it with Start.
        assert_eq!(wanted_running_is_idle(S::Stopped, None, AgentDesiredState::Running, false), (S::Stopped, None));
        // The other states keep their place, and their reason.
        for kept in [S::Error, S::Limited, S::SignedOut, S::WaitingOnUser, S::Working, S::Idle] {
            let (state, detail) = wanted_running_is_idle(kept, Some("why".into()), AgentDesiredState::Running, true);
            assert_eq!((state, detail.as_deref()), (kept, Some("why")));
        }
    }

    #[test]
    fn a_status_gives_the_state_its_reason_its_reset_and_what_is_wanted() {
        use crate::backend::agent_status::{Pane, Reach};
        let status = |availability, wanted, controller| AgentStatus { availability, pane: Pane::Parked, wanted, api: Reach::Online, controller };
        let stopped = Availability::NotRunning { exited_with_error: false };
        let (off, off_wanted) = (AgentRuntimeState::Stopped, AgentDesiredState::Stopped);
        assert_eq!(runtime_of(&status(stopped, Wanted::Running, true)), (AgentRuntimeState::Idle, Some(STARTS_ON_FIRST_MESSAGE.into()), None, AgentDesiredState::Running));
        assert_eq!(runtime_of(&status(stopped, Wanted::Running, false)), (off, None, None, AgentDesiredState::Running), "no controller: stays stopped");
        assert_eq!(runtime_of(&status(stopped, Wanted::Stopped, true)), (off, None, None, off_wanted), "a shut-down agent stays off");
        assert_eq!(runtime_of(&status(stopped, Wanted::Unset, true)), (off, None, None, off_wanted));
        let limited = Availability::Limited { resets_at_ms: Some(9_000), since_ms: 7 };
        assert_eq!(runtime_of(&status(limited, Wanted::Unset, true)), (AgentRuntimeState::Limited, Some("usage_limit".into()), Some(9_000), AgentDesiredState::Running));
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
    fn an_agent_begins_waiting_once_and_only_with_a_block() {
        let mut waiting = row("a", AgentRuntimeState::WaitingOnUser);
        waiting.block_id = Some("blk".into());
        assert!(begins_waiting(Some(AgentRuntimeState::Working), &waiting));
        assert!(begins_waiting(None, &waiting), "first seen already waiting");
        assert!(!begins_waiting(Some(AgentRuntimeState::WaitingOnUser), &waiting), "it was already waiting");
        waiting.block_id = None;
        assert!(!begins_waiting(Some(AgentRuntimeState::Working), &waiting), "no block, nothing to notify about");
        assert!(!begins_waiting(None, &row("b", AgentRuntimeState::Working)));
    }

    #[test]
    fn a_wait_ends_in_the_block_it_began_in() {
        let waiting = (AgentRuntimeState::WaitingOnUser, Some("blk".to_string()));
        let mut now = row("a", AgentRuntimeState::Working);
        assert_eq!(ends_waiting(Some(&waiting), &now).as_deref(), Some("blk"));
        now.block_id = Some("other".into());
        assert_eq!(ends_waiting(Some(&waiting), &now).as_deref(), Some("blk"), "the old block, not the row's");
        assert_eq!(ends_waiting(Some(&waiting), &row("a", AgentRuntimeState::WaitingOnUser)), None, "still waiting");
        assert_eq!(ends_waiting(Some(&(AgentRuntimeState::Working, Some("blk".into()))), &now), None, "was not waiting");
        assert_eq!(ends_waiting(None, &now), None);
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
