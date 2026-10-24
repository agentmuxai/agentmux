// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One notice per account episode, and the move to another account
//! (`docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §6).
//!
//! An **episode** is every agent pane that is `limited` or `signed_out` on one
//! account (`agent_state::unavailable_of_block`, which reads the account its
//! run launched on through `run_accounts_of_block`), from the first until none
//! is: the reset passed, the agents moved, or they recovered. srv keeps them
//! in memory ([`machine::Episodes`]) and rebuilds them from block meta on
//! start; a failure stored before the start is the episode rebuilt, not news.
//!
//! **What follows** from an episode:
//! - the `accountavailability` event when it opens, changes or closes (`ok`),
//!   and `identity.account.availability` for the open ones, which the Kanban
//!   and Swarm strips show with the move action;
//! - one OS toast (`NotifyKind::AccountUnavailable`) for a sign-in problem or
//!   a weekly (or unknown) limit, never for a session limit, and no per-pane
//!   `AgentCrashed` or "finished" toast for a pane it covers ([`covers_block`]);
//! - one `[AgentMux]` note to the agent that requested each task a newly
//!   blocked agent holds. A worker's leader already hears from
//!   `backend::worker_notes`.
//!
//! `identity.account.move_agents` ([`moves`]) links an episode's idle agents
//! to another signed-in account of the same provider and restarts their
//! panes; any link change clears the failure its old account caused
//! ([`failed_panes`], [`after_rebind`]).

mod machine;
mod moves;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::agents::failure::FailureClass;
use crate::backend::agent_state::{self, failure_of_block, run_accounts_of_block, AgentState};
use crate::backend::eventbus::EventBus;
use crate::backend::mps::MuxEvent;
use crate::backend::obj::{self, Block, Tab};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{NoArgsReq, COMMAND_IDENTITY_ACCOUNT_AVAILABILITY, COMMAND_IDENTITY_ACCOUNT_MOVE_AGENTS};
use crate::backend::storage::store::Store;
use crate::server::AppState;

use machine::{BlockedPane, Effect, Episode, Episodes};

/// The event an episode's change is published as.
pub const EVENT_ACCOUNT_AVAILABILITY: &str = "accountavailability";

/// How often the episodes are rebuilt without a prompt: a reset passing
/// changes them with no event.
const TICK: Duration = Duration::from_secs(15);
/// After a failure or a turn end, how long to wait for the other to land.
const SETTLE: Duration = Duration::from_millis(1_500);

/// One agent of an episode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountEpisodeAgent {
    pub block_id: String,
    /// Its definition id: the UID a Kanban task's holder carries.
    pub agent_id: String,
    pub agent_name: String,
}

/// A Kanban task an episode's agent holds (claimed, or blocked on a question).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountEpisodeTask {
    pub id: String,
    #[ts(type = "number")]
    pub number: i64,
    pub title: String,
}

/// An account the agents can move to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountMoveCandidate {
    pub account_id: String,
    pub label: String,
    /// Its last known sign-in status (`valid`, `ok`, `unknown`, …).
    pub status: String,
}

/// One account's episode, as `identity.account.availability` lists it for the
/// operator's windows. The label never goes in a toast, an event or to
/// another host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountAvailability {
    pub account_id: String,
    pub account_label: String,
    pub provider: String,
    #[ts(type = r#""limited" | "signed_out" | "ok""#)]
    pub state: String,
    /// When the last of its limits lifts (unix ms), when known.
    #[ts(type = "number | null")]
    pub resets_at_ms: Option<u64>,
    /// The provider's name for that limit's window (`five_hour`, `seven_day`, …).
    pub limit_window: Option<String>,
    /// When the episode opened (unix ms).
    #[ts(type = "number")]
    pub since_ms: u64,
    pub agents: Vec<AccountEpisodeAgent>,
    pub tasks: Vec<AccountEpisodeTask>,
    /// Where the agents can move, the first valid one first.
    pub candidates: Vec<AccountMoveCandidate>,
    /// The agents it covers, by definition id and by pane, so a view can tell
    /// whether a task's holder is in it without another call (the Kanban's
    /// Needs-you count, which counts an episode once, not each holder).
    pub agent_ids: Vec<String>,
    pub block_ids: Vec<String>,
}

/// `accountavailability` (spec §6.1): an episode opened, changed or closed
/// (`ok`). A signal for the windows, which then read
/// `identity.account.availability`: it carries no account label and no move
/// candidates, since any connection can subscribe to an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountAvailabilityEvent {
    pub account_id: String,
    pub provider: String,
    #[ts(type = r#""limited" | "signed_out" | "ok""#)]
    pub state: String,
    #[ts(type = "number | null")]
    pub resets_at_ms: Option<u64>,
    pub agents: Vec<AccountEpisodeAgent>,
    pub tasks: Vec<AccountEpisodeTask>,
}

impl From<AccountAvailability> for AccountAvailabilityEvent {
    fn from(a: AccountAvailability) -> Self {
        AccountAvailabilityEvent { account_id: a.account_id, provider: a.provider, state: a.state, resets_at_ms: a.resets_at_ms, agents: a.agents, tasks: a.tasks }
    }
}

/// `identity.account.availability`: the open episodes.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct IdentityAccountAvailabilityResult {
    pub episodes: Vec<AccountAvailability>,
}

use moves::IdentityAccountMoveAgentsReq;

struct Runtime {
    state: AppState,
    episodes: Mutex<Episodes>,
    wake: tokio::sync::Notify,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Start following the episodes. Call once at startup.
pub fn install(state: AppState) {
    let baseline = baseline(&state.mstore);
    let broker = state.broker.clone();
    if RUNTIME.set(Runtime { state, episodes: Mutex::new(Episodes::new(baseline)), wake: tokio::sync::Notify::new() }).is_err() {
        return;
    }
    // Observers run inside `Broker::publish`, under its locks: only a poke.
    broker.add_observer(Arc::new(|ev: &MuxEvent| {
        if ev.event == crate::backend::mps::EVENT_AGENT_FAILURE || ev.event == crate::backend::mps::EVENT_AGENT_TURN {
            refresh_soon();
        }
    }));
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async {
        let Some(rt) = RUNTIME.get() else { return };
        loop {
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = rt.wake.notified() => tokio::time::sleep(SETTLE).await,
            }
            let _ = tokio::task::spawn_blocking(refresh).await;
        }
    });
}

/// Rebuild the episodes soon (a failure, a turn end, a rebind).
pub(crate) fn refresh_soon() {
    if let Some(rt) = RUNTIME.get() {
        rt.wake.notify_one();
    }
}

/// Rebuild the episodes now and act on what changed. BLOCKING (store reads).
fn refresh() {
    let Some(rt) = RUNTIME.get() else { return };
    let now = agentmux_common::time::now_ms_u64();
    let panes = scan(&rt.state, now);
    let effects = rt.episodes.lock().unwrap_or_else(|e| e.into_inner()).update(panes, now);
    for effect in effects {
        apply(&rt.state, effect);
    }
}

/// The failure kinds an episode is made of: a usage limit or a refused sign-in.
fn blocking_failure(block: &Block) -> Option<agent_state::StoredFailure> {
    failure_of_block(block).filter(|f| matches!(f.code, FailureClass::UsageLimit | FailureClass::Auth))
}

fn signature(f: &agent_state::StoredFailure) -> String {
    format!("{:?}|{:?}|{}", f.code, f.resets_at_ms, f.title)
}

/// Every pane in a tab.
fn tab_blocks(mstore: &Store) -> Vec<Block> {
    let Ok(tabs) = mstore.get_all::<Tab>() else {
        return Vec::new();
    };
    tabs.iter().flat_map(|t| t.blockids.iter()).filter_map(|id| mstore.get::<Block>(id).ok().flatten()).collect()
}

/// The failures already stored when srv starts, whether or not their panes
/// have a controller yet.
fn baseline(mstore: &Store) -> HashMap<String, String> {
    tab_blocks(mstore).into_iter().filter_map(|b| blocking_failure(&b).map(|f| (b.oid.clone(), signature(&f)))).collect()
}

/// `p` canonical, or as given when it isn't a known provider.
fn canonical(provider: &str) -> String {
    match crate::backend::providers::resolve_provider_alias(provider) {
        "" => provider.to_lowercase(),
        c => c.to_string(),
    }
}

fn agent_name(mstore: &Store, block: &Block) -> String {
    let name = obj::meta_get_string(&block.meta, "agentName", "");
    if !name.trim().is_empty() {
        return name;
    }
    let def_id = obj::meta_get_string(&block.meta, "agentId", "");
    mstore
        .agent_def_list()
        .ok()
        .and_then(|defs| defs.into_iter().find(|d| d.id == def_id))
        .map(|d| d.name)
        .unwrap_or(def_id)
}

/// Every agent pane that is `limited` or `signed_out` on a known account now.
fn scan(state: &AppState, now_ms: u64) -> Vec<BlockedPane> {
    let accounts = state.id_store.identity_list(None).unwrap_or_default();
    let mut out = Vec::new();
    for block in tab_blocks(&state.mstore) {
        let Some(failure) = blocking_failure(&block) else { continue };
        let Some(u) = agent_state::unavailable_of_block(&state.mstore, Some(&state.identity_store), &block.oid, now_ms) else {
            continue;
        };
        let Some(account_id) = u.account_id.clone() else { continue };
        let provider = accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.provider.clone())
            .unwrap_or_else(|| obj::meta_get_string(&block.meta, "agentProvider", ""));
        let limit_window = block
            .meta
            .get(crate::backend::blockcontroller::core::META_LAST_FAILURE)
            .and_then(|f| f.get("limitWindow"))
            .and_then(|w| w.as_str())
            .map(str::to_string);
        out.push(BlockedPane {
            agent_id: obj::meta_get_string(&block.meta, "agentId", ""),
            agent_name: agent_name(&state.mstore, &block),
            block_id: block.oid.clone(),
            account_id,
            provider: canonical(&provider),
            state: u.state,
            resets_at_ms: u.resets_at_ms,
            limit_window,
            signature: signature(&failure),
        });
    }
    out
}

/// Whether an account episode speaks for this pane's failure, so its own
/// `AgentCrashed` or "finished" toast is left out (spec §6.2): its stored
/// failure is a usage limit that hasn't reset or a refused sign-in, on an
/// account. A pane with no account keeps its own toast. BLOCKING.
pub fn covers_block(block_id: &str) -> bool {
    let Some(rt) = RUNTIME.get() else { return false };
    covers(&rt.state.mstore, &rt.state.identity_store, block_id, agentmux_common::time::now_ms_u64())
}

/// [`covers_block`] against these stores at `now_ms`.
fn covers(mstore: &Store, identity: &Store, block_id: &str, now_ms: u64) -> bool {
    let Some(block) = mstore.get::<Block>(block_id).ok().flatten() else { return false };
    match blocking_failure(&block) {
        Some(f) if f.code == FailureClass::UsageLimit && f.resets_at_ms.is_some_and(|r| r <= now_ms) => false,
        Some(_) => !run_accounts_of_block(identity, &block).is_empty(),
        None => false,
    }
}

/// The toast's key for an account: its group, and the id it is taken down by.
/// Hashed, so the account id isn't in the notification.
fn toast_key(account_id: &str) -> String {
    format!("account:{}", crate::backend::notify::policy::tag_for(account_id))
}

fn apply(state: &AppState, effect: Effect) {
    match effect {
        Effect::Changed(ep) => {
            tracing::info!(provider = %ep.provider, agents = ep.panes.len(), state = ep.state().as_str(), "account episode: changed");
            publish(state, AccountAvailabilityEvent::from(view(state, &ep)));
        }
        Effect::Closed { account_id, provider } => {
            tracing::info!(provider = %provider, "account episode: closed");
            let ok = AccountAvailabilityEvent {
                account_id: account_id.clone(),
                provider,
                state: "ok".to_string(),
                resets_at_ms: None,
                agents: Vec::new(),
                tasks: Vec::new(),
            };
            publish(state, ok);
            if let Some(router) = crate::backend::notify::router::get(&state.broker) {
                router.resolve(&toast_key(&account_id), crate::backend::notify::policy::Family::Account);
            }
        }
        Effect::Toast(ep) => match crate::backend::notify::router::get(&state.broker) {
            Some(router) => router.account_unavailable(&toast_key(&ep.account_id), machine::toast_body(&ep)),
            None => tracing::info!("account episode: no notification router yet (no window connected); the strip shows it"),
        },
        Effect::Announce { episode, panes } => announce(state, &episode.account_id, &panes),
    }
}

fn publish(state: &AppState, payload: AccountAvailabilityEvent) {
    state.broker.publish_event::<crate::backend::mps_events::AccountAvailability>(&payload, vec![], 0);
}

fn label_of(state: &AppState, account_id: &str) -> String {
    state
        .id_store
        .identity_list(None)
        .unwrap_or_default()
        .iter()
        .find(|a| a.id == account_id)
        .map(crate::server::app_api::agent_spawn::account_label)
        .unwrap_or_else(|| "an account no longer in AgentMux".to_string())
}

/// The claimed or blocked tasks each of `block_ids` holds, by block, and who
/// requested each one.
fn held_tasks(state: &AppState, block_ids: &[&str]) -> HashMap<String, Vec<(AccountEpisodeTask, String)>> {
    use crate::backend::storage::work_board::requested;
    use crate::backend::storage::work_queue::work_state;
    let regs = state.reactive_handler.list_agents();
    let now = agentmux_common::time::now_ms();
    let tasks = state
        .identity_store
        .work_board_list(now - crate::backend::kanban::FINISHED_WINDOW_MS, 500, now)
        .unwrap_or_default();
    let mut out: HashMap<String, Vec<(AccountEpisodeTask, String)>> = HashMap::new();
    for t in tasks.iter().filter(|t| t.item.state == work_state::CLAIMED || t.item.state == work_state::BLOCKED) {
        let holder = regs.iter().find(|r| {
            if t.item.claimed_by_uid.is_empty() {
                !t.item.claimed_by.is_empty() && r.agent_id.eq_ignore_ascii_case(&t.item.claimed_by)
            } else {
                r.uid.as_deref() == Some(t.item.claimed_by_uid.as_str())
            }
        });
        let Some(holder) = holder.filter(|h| block_ids.contains(&h.block_id.as_str())) else { continue };
        let requester = if t.card.requested_by_kind == requested::AGENT && !t.card.requested_by.eq_ignore_ascii_case(&holder.agent_id) {
            t.card.requested_by.clone()
        } else {
            String::new()
        };
        let task = AccountEpisodeTask { id: t.item.id.clone(), number: t.card.num, title: t.item.title.clone() };
        out.entry(holder.block_id.clone()).or_default().push((task, requester));
    }
    out
}

/// What the strips and the event show for `ep`.
fn view(state: &AppState, ep: &Episode) -> AccountAvailability {
    let ids: Vec<&str> = ep.panes.iter().map(|p| p.block_id.as_str()).collect();
    let held = held_tasks(state, &ids);
    let mut tasks: Vec<AccountEpisodeTask> = ids.iter().flat_map(|b| held.get(*b).into_iter().flatten().map(|(t, _)| t.clone())).collect();
    tasks.sort_by_key(|t| t.number);
    tasks.dedup_by(|a, b| a.id == b.id);
    AccountAvailability {
        account_id: ep.account_id.clone(),
        account_label: label_of(state, &ep.account_id),
        provider: ep.provider.clone(),
        state: ep.state().as_str().to_string(),
        resets_at_ms: ep.resets_at_ms(),
        limit_window: ep.limit_window(),
        since_ms: ep.opened_at_ms,
        agents: ep
            .panes
            .iter()
            .map(|p| AccountEpisodeAgent { block_id: p.block_id.clone(), agent_id: p.agent_id.clone(), agent_name: p.agent_name.clone() })
            .collect(),
        tasks,
        candidates: moves::candidates(state, &ep.account_id, &ep.provider),
        agent_ids: {
            let mut ids: Vec<String> = ep.panes.iter().map(|p| p.agent_id.clone()).filter(|a| !a.is_empty()).collect();
            ids.sort();
            ids.dedup();
            ids
        },
        block_ids: ep.panes.iter().map(|p| p.block_id.clone()).collect(),
    }
}

/// For each requester of a task the newly blocked `panes` hold: the panes,
/// and the numbers of that requester's tasks each one holds.
fn requester_items(state: &AppState, panes: &[BlockedPane]) -> std::collections::BTreeMap<String, Vec<(BlockedPane, Vec<i64>)>> {
    let ids: Vec<&str> = panes.iter().map(|p| p.block_id.as_str()).collect();
    let held = held_tasks(state, &ids);
    let mut out: std::collections::BTreeMap<String, Vec<(BlockedPane, Vec<i64>)>> = Default::default();
    for pane in panes {
        let mut by_requester: std::collections::BTreeMap<String, Vec<i64>> = Default::default();
        for (task, requester) in held.get(&pane.block_id).into_iter().flatten() {
            if !requester.is_empty() {
                by_requester.entry(requester.clone()).or_default().push(task.number);
            }
        }
        for (requester, mut numbers) in by_requester {
            numbers.sort_unstable();
            out.entry(requester).or_default().push((pane.clone(), numbers));
        }
    }
    out
}

/// The notes to send for the newly blocked `panes` of `account_id`'s episode:
/// one per requester per episode (spec §6.2), naming each of its agents and
/// tasks. A requester already told in this episode gets nothing more when
/// another agent joins: the strip shows the rest.
fn requester_notes(state: &AppState, episodes: &Mutex<Episodes>, account_id: &str, panes: &[BlockedPane]) -> Vec<(String, String)> {
    let items = requester_items(state, panes);
    let fresh = episodes.lock().unwrap_or_else(|e| e.into_inner()).first_notes(account_id, items.keys().cloned());
    items
        .into_iter()
        .filter(|(requester, _)| fresh.contains(requester))
        .map(|(requester, items)| {
            let refs: Vec<(&BlockedPane, &[i64])> = items.iter().map(|(p, n)| (p, n.as_slice())).collect();
            let text = format!("[AgentMux] {}", crate::backend::reactive::sanitize::quote_in_system_note(&machine::leader_note(&refs)));
            (requester, text)
        })
        .collect()
}

fn announce(state: &AppState, account_id: &str, panes: &[BlockedPane]) {
    let Some(rt) = RUNTIME.get() else { return };
    for (requester, text) in requester_notes(state, &rt.episodes, account_id, panes) {
        if let Err(e) = state.reactive_handler.deliver_system_note(&requester, &text) {
            tracing::info!(requester = %requester, error = %e, "account episode: requester note not delivered");
        }
    }
}

/// The open episodes, as the strips show them. BLOCKING.
fn availability(state: &AppState) -> IdentityAccountAvailabilityResult {
    let open: Vec<Episode> = match RUNTIME.get() {
        Some(rt) => rt.episodes.lock().unwrap_or_else(|e| e.into_inner()).list().cloned().collect(),
        None => Vec::new(),
    };
    IdentityAccountAvailabilityResult { episodes: open.iter().map(|ep| view(state, ep)).collect() }
}

/// `agent_id`'s panes for `provider` whose stored failure is a usage limit
/// or a refused sign-in, with the accounts their runs count against. Read
/// before a link changes: a pane from before `agent:run_account` counts
/// against the agent's links, which the change replaces.
pub fn failed_panes(mstore: &Store, identity: &Store, agent_id: &str, provider: &str) -> Vec<(String, Vec<String>)> {
    let provider = canonical(provider);
    tab_blocks(mstore)
        .into_iter()
        .filter(|b| obj::meta_get_string(&b.meta, "agentId", "") == agent_id)
        .filter(|b| canonical(&obj::meta_get_string(&b.meta, "agentProvider", "")) == provider)
        .filter(|b| blocking_failure(b).is_some())
        .map(|b| {
            let accounts = run_accounts_of_block(identity, &b);
            (b.oid, accounts)
        })
        .collect()
}

/// An agent was linked to `account_id` (spec §6.3, "on rebind"): clear the
/// stored failure of each of `before` ([`failed_panes`]) whose run wasn't on
/// that account, then wake held messages and the episodes. Returns how many
/// were cleared.
pub fn after_rebind(mstore: &Arc<Store>, event_bus: &Arc<EventBus>, before: Vec<(String, Vec<String>)>, account_id: &str) -> usize {
    let mut cleared = 0;
    for (block_id, accounts) in before {
        if accounts.iter().any(|a| a == account_id) {
            continue;
        }
        crate::backend::blockcontroller::core::persist_last_failure(&block_id, None, &Some(mstore.clone()), &Some(event_bus.clone()));
        cleared += 1;
    }
    if cleared > 0 {
        tracing::info!(cleared, "account episode: a rebind cleared its old account's failures");
        crate::backend::reactive::held_jekt_wake().notify_one();
        refresh_soon();
    }
    cleared
}

/// Both RPCs are the operator's: an agent connection (`bus:register`) is
/// refused, since the answer names accounts and the move relinks and restarts
/// other agents.
pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_IDENTITY_ACCOUNT_AVAILABILITY, move |_req: Option<NoArgsReq>, ctx| {
        let s = s.clone();
        async move {
            ctx.operator_only("only the operator sees which accounts their agents can't run on")?;
            tokio::task::spawn_blocking(move || availability(&s)).await.map_err(|e| e.to_string())
        }
    });
    let s = state.clone();
    engine.register_typed(COMMAND_IDENTITY_ACCOUNT_MOVE_AGENTS, move |req: IdentityAccountMoveAgentsReq, ctx| {
        let s = s.clone();
        async move {
            ctx.operator_only("only the operator moves agents to another account")?;
            let result = tokio::task::spawn_blocking(move || moves::move_agents(&s, &req)).await.map_err(|e| e.to_string())??;
            refresh_soon();
            Ok(result)
        }
    });
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
