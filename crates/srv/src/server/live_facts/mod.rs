// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an agent is running on and doing now, read from AgentMux's own runtime state and never from the
//! agent's say-so (docs/specs/SPEC_MUXAPI_AGENT_LIVE_FACTS_2026_11_20.md). One builder, [`facts_of`], for
//! `Agents list`, `Agents status` and the Swarm's rows.
//!
//! The model and effort come first (`model`, the one group every row carries in short form); the rest are
//! groups a caller asks for by name (`Select`), so a default row stays small and each group costs only
//! when asked. Never in a fact: a credential, a vault entry, a prompt or a message (spec §3, "Not exposed").

pub mod handler;
pub mod model;
pub mod query;

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use crate::backend::account_usage::AccountUsage;
use crate::backend::agent_runtime::{spawn_runtime_from_args, AgentRuntimeEvent};
use crate::backend::agent_status::Pane;
use crate::backend::obj::Block;
use crate::backend::storage::identities::IdentityAccount;
use crate::backend::storage::AgentDefinition;
use crate::server::AppState;

pub use model::ModelFacts;

/// A context reading with no new call for this long, in a turn, is stale.
const CONTEXT_STALE_MS: u64 = 5 * 60_000;

/// Where an agent runs and how it is set up.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct RuntimeFacts {
    /// The provider's harness (`claude`, `codex`, ...).
    pub harness: String,
    /// `native` (a process on its host) or `container`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub permission_mode: Option<String>,
    /// Its pane is in a tab an open window shows.
    pub pane_shown: bool,
}

/// Whether the agent is in a turn, and since when.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TurnFacts {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub since_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub last_activity_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub idle_ms: Option<u64>,
}

/// How full the agent's context is. In memory: absent until the agent's next frame after srv starts.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ContextFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub auto_compact_at: Option<u64>,
    pub compacting: bool,
    #[ts(type = "number")]
    pub compactions: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub observed_ms: Option<u64>,
    /// In a turn with no new reading for a while.
    pub stale: bool,
}

/// The account the agent's run is on: a stable id and the label Identity accounts shows. Never a credential.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AccountFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stable_id: Option<String>,
    pub label: String,
}

/// The Kanban tasks the agent holds now.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct TaskFacts {
    /// The first of them.
    #[ts(type = "number")]
    pub number: i64,
    #[ts(type = "Array<number>")]
    pub all: Vec<i64>,
}

/// An agent that owes work and has done nothing for the quiet time
/// (SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md §6): the Swarm's and the Kanban's "quiet 22 min". Read from
/// the last sweep, so it can be up to half a minute behind; absent while the agent is not quiet.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct QuietFacts {
    /// When the quiet began: the agent's last activity.
    #[ts(type = "number")]
    pub since_ms: u64,
    /// How long it has been quiet.
    #[ts(type = "number")]
    pub quiet_ms: u64,
    /// The Kanban tasks it holds; empty for a spawned worker that owes its leader a report.
    #[ts(type = "Array<number>")]
    pub tasks: Vec<i64>,
    /// The agents told about it (names).
    pub watchers: Vec<String>,
}

/// When the agent's process started.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SessionFacts {
    #[ts(type = "number")]
    pub started_ms: u64,
    #[ts(type = "number")]
    pub uptime_ms: u64,
}

/// The AgentMux the agent runs in.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct HostFacts {
    pub name: String,
    pub version: String,
    pub build: String,
    pub channel: String,
}

/// The groups asked for; each is absent when not asked for or when nothing says.
#[derive(Debug, Clone, Default, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct LiveFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<ModelFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub runtime: Option<RuntimeFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn: Option<TurnFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub context: Option<ContextFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub account: Option<AccountFacts>,
    /// The account's usage windows, as `Identity accounts` reads them (backend::account_usage).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub usage: Option<AccountUsage>,
    /// What a route agent has spent today and its cap, as the list row's `spend` says.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub spend: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub task: Option<TaskFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub quiet: Option<QuietFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub session: Option<SessionFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<HostFacts>,
}

/// Which groups a caller asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Select {
    pub model: bool,
    pub runtime: bool,
    pub turn: bool,
    pub context: bool,
    pub account: bool,
    pub usage: bool,
    pub spend: bool,
    pub task: bool,
    pub quiet: bool,
    pub session: bool,
    pub host: bool,
}

impl Select {
    pub const GROUPS: [&'static str; 11] = ["model", "runtime", "turn", "context", "account", "usage", "spend", "task", "quiet", "session", "host"];

    /// Every group (`host` only when `with_host`).
    pub fn all(with_host: bool) -> Self {
        Self { model: true, runtime: true, turn: true, context: true, account: true, usage: true, spend: true, task: true, quiet: true, session: true, host: with_host }
    }

    fn set(&mut self, group: &str) -> bool {
        match group {
            "model" => self.model = true,
            "runtime" => self.runtime = true,
            "turn" => self.turn = true,
            "context" => self.context = true,
            "account" => self.account = true,
            "usage" => self.usage = true,
            "spend" => self.spend = true,
            "task" => self.task = true,
            "quiet" => self.quiet = true,
            "session" => self.session = true,
            "host" => self.host = true,
            _ => return false,
        }
        true
    }

    /// From `fields` (comma-separated group names, or `all`) and `detail` (`basic`, `facts`, `full`).
    pub fn parse(fields: Option<&str>, detail: Option<&str>) -> Result<Self, String> {
        let mut sel = match detail.map(str::trim).filter(|d| !d.is_empty()) {
            None | Some("basic") => Self::default(),
            Some("facts") => Self::all(false),
            Some("full") => Self::all(true),
            Some(other) => return Err(format!("unknown detail {other:?}: use basic, facts or full")),
        };
        for name in fields.unwrap_or_default().split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if name == "all" {
                sel = Self::all(true);
            } else if !sel.set(name) {
                return Err(format!("unknown field group {name:?}: the groups are {}, or all", Self::GROUPS.join(", ")));
            }
        }
        Ok(sel)
    }

    /// A `facts` object is wanted at all.
    pub fn any(&self) -> bool {
        *self != Self::default()
    }
}

/// What every agent of a call shares, read once.
pub struct Ctx<'a> {
    state: &'a AppState,
    now_ms: u64,
    defs: Vec<AgentDefinition>,
    /// Kanban task numbers held, by the holder's uid or `name:<lowercase name>`.
    claimed: HashMap<String, Vec<i64>>,
    accounts: Vec<IdentityAccount>,
    settings: HashMap<String, Value>,
    net: crate::connectivity::ConnectivityStatus,
}

impl<'a> Ctx<'a> {
    /// Reads what `sel` and the filters will need: the claimed tasks only when tasks are asked for, the
    /// accounts only when an account or its usage is.
    pub fn new(state: &'a AppState, now_ms: u64, sel: &Select, needs_account: bool) -> Self {
        let defs = state.mstore.agent_def_list().unwrap_or_default();
        let accounts = if sel.account || sel.usage || needs_account { state.id_store.identity_list(None).unwrap_or_default() } else { Vec::new() };
        Self {
            state,
            now_ms,
            defs,
            claimed: if sel.task { claimed_tasks(state) } else { HashMap::new() },
            accounts,
            settings: state.config_watcher.get_settings().extra.clone(),
            net: crate::connectivity::status(),
        }
    }
}

/// The Kanban task numbers each agent holds.
fn claimed_tasks(state: &AppState) -> HashMap<String, Vec<i64>> {
    use crate::backend::storage::work_queue::work_state;
    let now = agentmux_common::time::now_ms();
    let items = state.work_store.work_board_list(now - crate::backend::kanban::FINISHED_WINDOW_MS, 500, now).unwrap_or_default();
    let mut out: HashMap<String, Vec<i64>> = HashMap::new();
    for t in items.iter().filter(|t| t.item.state == work_state::CLAIMED) {
        let key = if t.item.claimed_by_uid.is_empty() { format!("name:{}", t.item.claimed_by.to_lowercase()) } else { t.item.claimed_by_uid.clone() };
        out.entry(key).or_default().push(t.card.num);
    }
    out.values_mut().for_each(|v| v.sort_unstable());
    out
}

/// One agent, as a caller of [`facts_of`] knows it.
pub struct Agent<'a> {
    pub uid: &'a str,
    pub name: &'a str,
    pub provider: &'a str,
    pub block_id: Option<&'a str>,
    /// Its runtime state (`idle`, `working`, `stopped`, ...), as `agent_runtime` names it.
    pub state: &'a str,
    pub container: bool,
}

/// The agent's own block, when it has one.
fn block_of(ctx: &Ctx, a: &Agent) -> Option<Block> {
    a.block_id.and_then(|b| ctx.state.mstore.get::<Block>(b).ok().flatten())
}

/// The model and effort: the one group every row carries (short, as `model_label` and `model_source`).
pub fn model_of(ctx: &Ctx, a: &Agent, block: Option<&Block>) -> ModelFacts {
    let def = ctx.defs.iter().find(|d| d.id == a.uid);
    let event: Option<AgentRuntimeEvent> = a.block_id.and_then(crate::backend::agent_runtime_latest::get);
    let reading = a.block_id.and_then(crate::backend::context_reading::get);
    let choice = def.and_then(|d| crate::backend::model_gateway::launch::choice_of(&ctx.state.mstore, &d.id));
    let route = choice.as_ref().and_then(|c| {
        let r = crate::backend::model_gateway::catalog::route(&c.route)?;
        let m = r.model(&c.model)?;
        Some((r.id, r.label, m.id, m.label))
    });
    let meta = block.map(|b| &b.meta);
    let meta_runtime = meta.and_then(|m| m.get("agent:runtime"));
    let meta_model = meta_runtime.and_then(|r| r.get("model")).and_then(Value::as_str).or_else(|| meta.and_then(|m| m.get("agent:model")).and_then(Value::as_str));
    let definition_model = def.and_then(|d| {
        let argv: Vec<String> = d.provider_flags.split_whitespace().map(str::to_string).collect();
        spawn_runtime_from_args(&argv).model
    });
    model::resolve(&model::Inputs {
        event: event.as_ref(),
        observed: reading.as_ref().and_then(|r| r.model.as_deref()),
        route,
        meta_model,
        meta_effort: meta_runtime.and_then(|r| r.get("effort")).and_then(Value::as_str),
        definition_model: definition_model.as_deref(),
        running: a.state != "stopped" && a.state != "error",
    })
}

fn runtime_of(ctx: &Ctx, a: &Agent, block: Option<&Block>) -> RuntimeFacts {
    let def = ctx.defs.iter().find(|d| d.id == a.uid);
    let event = a.block_id.and_then(crate::backend::agent_runtime_latest::get);
    let meta_mode = block.and_then(|b| b.meta.get("agent:runtime")).and_then(|r| r.get("permission_mode")).and_then(Value::as_str);
    let pane = super::availability_view::status_of(&ctx.state.mstore, a.block_id, block, &ctx.net, ctx.now_ms).pane;
    RuntimeFacts {
        harness: a.provider.to_string(),
        kind: if a.container { "container" } else { "native" }.to_string(),
        image: a.container.then(|| def.map(|d| crate::backend::container::image::resolve_container_image(&d.container_image))).flatten(),
        permission_mode: event.and_then(|e| e.permission_mode).or_else(|| meta_mode.map(str::to_string)),
        pane_shown: pane == Pane::Shown,
    }
}

fn turn_of(ctx: &Ctx, a: &Agent) -> Option<TurnFacts> {
    let ctrl = crate::backend::blockcontroller::get_controller(a.block_id?)?;
    let tracker = ctrl.turn_tracker()?;
    let ms = |v: Option<i64>| v.and_then(|v| u64::try_from(v).ok());
    let since = ms(tracker.pass_started_ms());
    let last = ms(ctrl.last_output_ms());
    Some(TurnFacts {
        active: since.is_some(),
        since_ms: since,
        duration_ms: since.map(|s| ctx.now_ms.saturating_sub(s)),
        last_activity_ms: last,
        idle_ms: last.map(|l| ctx.now_ms.saturating_sub(l)),
    })
}

fn context_of(ctx: &Ctx, a: &Agent, turn_active: bool) -> Option<ContextFacts> {
    let r = crate::backend::context_reading::get(a.block_id?)?;
    r.tokens.or(r.window)?;
    let percent = match (r.tokens, r.window) {
        (Some(t), Some(w)) if w > 0 => Some((t as f64 / w as f64 * 1000.0).round() / 10.0),
        _ => None,
    };
    Some(ContextFacts {
        tokens: r.tokens,
        window: r.window,
        percent,
        auto_compact_at: r.auto_compact_at,
        compacting: r.compacting,
        compactions: r.compactions,
        observed_ms: r.observed_ms,
        stale: turn_active && r.observed_ms.is_some_and(|o| ctx.now_ms.saturating_sub(o) > CONTEXT_STALE_MS),
    })
}

/// The account the pane's run is on (`agent:run_account`), with its stable id, label and usage.
fn account_of(ctx: &Ctx, block: Option<&Block>) -> Option<(AccountFacts, Option<AccountUsage>)> {
    let id = block?.meta.get(crate::backend::agent_state::RUN_ACCOUNT_META_KEY)?.as_str()?;
    let acct = ctx.accounts.iter().find(|a| a.id == id)?;
    let facts = AccountFacts {
        stable_id: crate::identity::stable_account::stable_account_id(acct),
        label: safe_label(acct),
    };
    Some((facts, Some(AccountUsage::of(&acct.provider, id, ctx.now_ms))))
}

/// The account's name for a reader of the fleet: its display name or its name, never an email (a name can be one, and the
/// account's own label falls back to the email) and never read from its context.
fn safe_label(a: &IdentityAccount) -> String {
    [a.display_name.as_str(), a.name.as_str()]
        .into_iter()
        .map(str::trim)
        .find(|s| !s.is_empty() && !s.contains('@'))
        .map_or_else(|| format!("{} account", a.provider), str::to_string)
}

fn task_of(ctx: &Ctx, a: &Agent) -> Option<TaskFacts> {
    let all = ctx.claimed.get(a.uid).or_else(|| ctx.claimed.get(&format!("name:{}", a.name.to_lowercase())))?;
    Some(TaskFacts { number: *all.first()?, all: all.clone() })
}

fn quiet_of(ctx: &Ctx, a: &Agent) -> Option<QuietFacts> {
    let q = crate::backend::quiet_watch::quiet_view(a.uid)?;
    Some(QuietFacts { since_ms: q.since_ms, quiet_ms: ctx.now_ms.saturating_sub(q.since_ms), tasks: q.tasks, watchers: q.watchers })
}

fn session_of(ctx: &Ctx, a: &Agent) -> Option<SessionFacts> {
    let tracker = crate::backend::blockcontroller::get_controller(a.block_id?)?.turn_tracker()?;
    let started = u64::try_from(tracker.spawned_at_ms()?).ok()?;
    Some(SessionFacts { started_ms: started, uptime_ms: ctx.now_ms.saturating_sub(started) })
}

fn host_of(ctx: &Ctx) -> HostFacts {
    HostFacts {
        name: ctx.state.hostname.clone(),
        version: ctx.state.version.clone(),
        build: std::env::var("AGENTMUX_BUILD").ok().filter(|b| !b.is_empty()).unwrap_or_else(|| ctx.state.version.clone()),
        channel: crate::backend::reactive::registry::local_channel_id(),
    }
}

/// The groups of `sel` for `a`. `model` is the answer [`model_of`] already gave, so it is read once.
pub fn facts_of(ctx: &Ctx, sel: &Select, a: &Agent, model: ModelFacts) -> LiveFacts {
    let block = if sel.runtime || sel.account || sel.usage { block_of(ctx, a) } else { None };
    let turn = (sel.turn || sel.context).then(|| turn_of(ctx, a)).flatten();
    let (account, usage) = if sel.account || sel.usage { account_of(ctx, block.as_ref()).unzip() } else { (None, None) };
    LiveFacts {
        model: sel.model.then_some(model),
        runtime: sel.runtime.then(|| runtime_of(ctx, a, block.as_ref())),
        context: sel.context.then(|| context_of(ctx, a, turn.as_ref().is_some_and(|t| t.active))).flatten(),
        turn: sel.turn.then_some(turn).flatten(),
        account: if sel.account { account } else { None },
        usage: if sel.usage { usage.flatten() } else { None },
        spend: sel.spend.then(|| super::spend_watch::view(&ctx.state.mstore, &ctx.state.identity_store, &ctx.settings, &ctx.defs, a.uid)).flatten(),
        task: sel.task.then(|| task_of(ctx, a)).flatten(),
        quiet: sel.quiet.then(|| quiet_of(ctx, a)).flatten(),
        session: sel.session.then(|| session_of(ctx, a)).flatten(),
        host: sel.host.then(|| host_of(ctx)),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
