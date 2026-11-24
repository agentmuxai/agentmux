// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The parameters of `Agents list` and `Agents status` (spec §5): which groups, which agents, which page; and
//! the two answers built from them. Filters run on the cheap facts (state, model, provider, runtime, account)
//! for every agent; the groups asked for are read only for the agents of the page.

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};

use super::model::{ModelFacts, ModelSource};
use super::{block_of, facts_of, model_of, Agent, Ctx, LiveFacts, Select};
use crate::backend::runtime_adapter::RuntimeKind;
use crate::server::AppState;

/// A list that asks for groups or a page stops at this many bytes (a default call keeps every agent, as
/// it always has); `Agents status` is the compact way to see them all.
const DEFAULT_MAX_BYTES: usize = 60_000;

/// The agents a caller wants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// An `AgentRuntimeState` name: `idle`, `working`, `stopped`, ...
    pub state: Option<String>,
    /// Part of the model id or its label, case-insensitive; a leading `!` means "not this".
    pub model: Option<String>,
    pub provider: Option<String>,
    /// `native` or `container`.
    pub runtime: Option<String>,
    /// Part of the account's stable id or label.
    pub account: Option<String>,
    /// Part of the seat's role, lead or area.
    pub seat: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Params {
    pub select: Select,
    pub filter: Filter,
    pub limit: Option<usize>,
    pub offset: usize,
    pub group_by: Option<String>,
    /// `host: all` was asked: the answer also names the LAN peers, so the caller can ask each one.
    pub all_hosts: bool,
}

fn text(map: &HashMap<String, String>, key: &str) -> Option<String> {
    map.get(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl Params {
    /// From the query string of the route. `host` may only name this machine (reading another machine's
    /// agents is the LAN call).
    pub fn from_map(map: &HashMap<String, String>, this_host: &str) -> Result<Self, String> {
        let select = Select::parse(map.get("fields").map(String::as_str), map.get("detail").map(String::as_str))?;
        let host = text(map, "host");
        if let Some(host) = host.as_deref() {
            if !host.eq_ignore_ascii_case("all") && !host.eq_ignore_ascii_case(this_host) {
                return Err(format!("host {host:?} is not this machine ({this_host}): this route answers for this machine; another machine's agents are asked for with the tool's host argument"));
            }
        }
        let number = |key: &str| -> Result<Option<usize>, String> {
            text(map, key).map(|v| v.parse::<usize>().map_err(|_| format!("{key} must be a whole number, not {v:?}"))).transpose()
        };
        let group_by = text(map, "group_by");
        if let Some(g) = group_by.as_deref().filter(|g| !["model", "state", "host", "account", "provider", "runtime"].contains(g)) {
            return Err(format!("unknown group_by {g:?}: use model, state, host, account, provider or runtime"));
        }
        Ok(Self {
            select,
            filter: Filter {
                state: text(map, "state"),
                model: text(map, "model"),
                provider: text(map, "provider"),
                runtime: text(map, "runtime"),
                account: text(map, "account"),
                seat: text(map, "seat"),
            },
            limit: number("limit")?,
            offset: number("offset")?.unwrap_or(0),
            group_by,
            all_hosts: host.as_deref().is_some_and(|h| h.eq_ignore_ascii_case("all")),
        })
    }
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

/// Whether `model` is the one `wanted` names: part of its id or its label.
fn names_model(model: &ModelFacts, wanted: &str) -> bool {
    model.id.as_deref().is_some_and(|id| contains_ci(id, wanted)) || model.label.as_deref().is_some_and(|l| contains_ci(l, wanted))
}

/// What a filter reads of one agent.
pub struct Cheap<'a> {
    pub state: &'a str,
    pub model: &'a ModelFacts,
    pub provider: &'a str,
    pub container: bool,
    pub account: Option<(&'a str, &'a str)>,
    pub seat: &'a str,
}

/// Whether the agent is wanted. `!model` keeps an agent whose model is not known too: it is not shown to be that model.
pub fn wanted(f: &Filter, c: &Cheap) -> bool {
    let model_ok = match f.model.as_deref() {
        None => true,
        Some(m) => match m.strip_prefix('!') {
            Some(not) => c.model.source == ModelSource::Unknown || !names_model(c.model, not.trim()),
            None => names_model(c.model, m),
        },
    };
    model_ok
        && f.state.as_deref().is_none_or(|s| c.state.eq_ignore_ascii_case(s))
        && f.provider.as_deref().is_none_or(|p| c.provider.eq_ignore_ascii_case(p))
        && f.runtime.as_deref().is_none_or(|r| r.eq_ignore_ascii_case(if c.container { "container" } else { "native" }))
        && f.account.as_deref().is_none_or(|a| c.account.is_some_and(|(stable, label)| contains_ci(stable, a) || contains_ci(label, a)))
        && f.seat.as_deref().is_none_or(|s| contains_ci(c.seat, s))
}

fn string_at<'a>(row: &'a Value, pointer: &str) -> &'a str {
    row.pointer(pointer).and_then(Value::as_str).unwrap_or_default()
}

/// The agent a discovery row describes, when it has a runtime row to say its state.
fn agent_of(row: &Value) -> Option<Agent<'_>> {
    row.get("runtime").filter(|r| r.is_object())?;
    Some(Agent {
        uid: string_at(row, "/id"),
        name: string_at(row, "/name"),
        provider: string_at(row, "/runtime/provider"),
        block_id: row.pointer("/runtime/block_id").and_then(Value::as_str),
        state: string_at(row, "/runtime/state"),
        container: row.pointer("/runtime/runtime").and_then(|v| serde_json::from_value::<RuntimeKind>(v.clone()).ok()) == Some(RuntimeKind::Container),
    })
}

/// What a seat says, as words to match against: its role, qualifier, lead (name and id) and area. Not its JSON, whose
/// key names would match every agent that has a seat.
fn seat_words(seat: &Value) -> String {
    ["/role", "/qualifier", "/lead/name", "/lead/id", "/area"].iter().filter_map(|p| seat.pointer(p).and_then(Value::as_str)).collect::<Vec<_>>().join(" ")
}

/// One kept agent: its row, and what was read to keep it.
struct Kept {
    row: Value,
    model: Option<ModelFacts>,
}

/// The rows that pass the filter, each with its model answer (an agent with no runtime row passes an
/// empty filter only).
fn filtered(ctx: &Ctx, rows: Vec<Value>, p: &Params) -> Vec<Kept> {
    let narrowed = p.filter != Filter::default();
    rows.into_iter()
        .filter_map(|row| {
            let Some(a) = agent_of(&row) else { return (!narrowed).then_some(Kept { row, model: None }) };
            let block = block_of(ctx, &a);
            let model = model_of(ctx, &a, block.as_ref());
            let account = p.filter.account.as_ref().and_then(|_| super::account_of(ctx, block.as_ref()));
            let seat = row.get("seat").map(seat_words).unwrap_or_default();
            let cheap = Cheap {
                state: a.state,
                model: &model,
                provider: a.provider,
                container: a.container,
                account: account.as_ref().map(|(f, _)| (f.stable_id.as_deref().unwrap_or_default(), f.label.as_str())),
                seat: &seat,
            };
            wanted(&p.filter, &cheap).then_some(Kept { row, model: Some(model) })
        })
        .collect()
}

/// The page of `kept`, their groups read now, and a note of what was cut.
fn decorate_page(ctx: &Ctx, kept: Vec<Kept>, p: &Params) -> (Vec<Value>, Value) {
    let total = kept.len();
    let limit = p.limit.unwrap_or(usize::MAX);
    let bounded = p.limit.is_some() || p.select.any();
    let mut out: Vec<Value> = Vec::new();
    let mut bytes = 0usize;
    let mut truncated = false;
    for k in kept.into_iter().skip(p.offset).take(limit) {
        let Kept { mut row, model } = k;
        if let (Some(model), Some(a)) = (model, agent_of(&row).map(|a| (a.uid.to_string(), a.name.to_string(), a.provider.to_string(), a.block_id.map(str::to_string), a.state.to_string(), a.container))) {
            row["model_label"] = json!(model.label);
            row["model_source"] = json!(model.source);
            if p.select.any() {
                let agent = Agent { uid: &a.0, name: &a.1, provider: &a.2, block_id: a.3.as_deref(), state: &a.4, container: a.5 };
                row["facts"] = json!(facts_of(ctx, &p.select, &agent, model));
            }
        }
        bytes += row.to_string().len();
        if bounded && bytes > DEFAULT_MAX_BYTES && !out.is_empty() {
            truncated = true;
            break;
        }
        out.push(row);
    }
    let page = json!({ "total": total, "returned": out.len(), "offset": p.offset, "limit": p.limit, "truncated": truncated, "bytes": bytes });
    (out, page)
}

/// The rows of `Agents list`: filtered, paged, each with its short model and the groups asked for; and the page's note.
pub fn apply(state: &AppState, rows: Vec<Value>, p: &Params, now_ms: u64) -> (Vec<Value>, Value) {
    let ctx = Ctx::new(state, now_ms, &p.select, p.filter.account.is_some());
    let kept = filtered(&ctx, rows, p);
    decorate_page(&ctx, kept, p)
}

/// "4m", "2h 5m", "30s": a duration a person reads at a glance.
pub fn short_duration(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        _ => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

/// "5h 60% 7d 44%": the account's windows, the one that resets first first.
fn usage_short(u: &crate::backend::account_usage::AccountUsage) -> String {
    let mut windows: Vec<(&String, &crate::backend::account_usage::UsageWindow)> = u.windows.iter().collect();
    windows.sort_by_key(|(_, w)| w.resets_at_ms);
    let name = |k: &str| match k {
        "five_hour" => "5h".to_string(),
        "seven_day" => "7d".to_string(),
        other => other.replace('_', " "),
    };
    windows.iter().map(|(k, w)| format!("{} {}%", name(k), w.used_percent.round())).collect::<Vec<_>>().join(" ")
}

/// `name | state (turn 4m), quiet 22m | Sonnet 5.5 high | ctx 62% | account | 5h 60% 7d 44% | #1234`: what a leader reads to see the
/// fleet. A fact that is not known is left out, not guessed.
pub fn status_line(name: &str, state: &str, facts: &LiveFacts, model: &ModelFacts) -> String {
    let mut parts: Vec<String> = vec![name.to_string()];
    let turn = facts.turn.as_ref().filter(|t| t.active).and_then(|t| t.duration_ms).map(|d| format!(" (turn {})", short_duration(d))).unwrap_or_default();
    let quiet = facts.quiet.as_ref().map(|q| format!(", quiet {}", short_duration(q.quiet_ms))).unwrap_or_default();
    parts.push(format!("{state}{turn}{quiet}"));
    let mut m = model.label.clone().or_else(|| model.id.clone()).unwrap_or_else(|| "model unknown".to_string());
    if let Some(e) = &model.effort {
        m = format!("{m} {e}");
    }
    if model.source == ModelSource::Requested || model.pending_switch {
        m.push('?');
    }
    parts.push(m);
    if let Some(p) = facts.context.as_ref().and_then(|c| c.percent) {
        parts.push(format!("ctx {}%", p.round()));
    }
    if let Some(a) = &facts.account {
        parts.push(a.label.clone());
    }
    if let Some(u) = facts.usage.as_ref().filter(|u| !u.windows.is_empty()) {
        parts.push(usage_short(u));
    }
    if let Some(t) = &facts.task {
        parts.push(format!("#{}", t.number));
    }
    parts.join(" | ")
}

/// How the compact answer groups its counts.
fn key_of(group: &str, name_state: &str, model: &ModelFacts, a: &Agent, facts: &LiveFacts, host: &str) -> String {
    match group {
        "model" => model.label.clone().or_else(|| model.id.clone()).unwrap_or_else(|| "unknown".to_string()),
        "state" => name_state.to_string(),
        "host" => host.to_string(),
        "account" => facts.account.as_ref().map_or_else(|| "none".to_string(), |x| x.label.clone()),
        "provider" => a.provider.to_string(),
        _ => if a.container { "container" } else { "native" }.to_string(),
    }
}

/// `Agents status`: one line per agent, the counts by state, model and host, and, with `group_by`, the agents grouped.
pub fn status(state: &AppState, rows: Vec<Value>, p: &Params, now_ms: u64) -> Value {
    let mut sel = p.select;
    for group in ["model", "turn", "context", "account", "usage", "task", "quiet"] {
        let _ = sel.set(group);
    }
    let p = Params { select: sel, ..p.clone() };
    let ctx = Ctx::new(state, now_ms, &p.select, p.filter.account.is_some());
    let kept = filtered(&ctx, rows, &p);
    let total = kept.len();
    let host = state.hostname.clone();
    let (mut lines, mut by_state, mut by_model, mut groups): (Vec<String>, BTreeMap<String, usize>, BTreeMap<String, usize>, BTreeMap<String, Vec<String>>) =
        (Vec::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for k in kept.into_iter().skip(p.offset).take(p.limit.unwrap_or(usize::MAX)) {
        let Some(a) = agent_of(&k.row) else { continue };
        let Some(model) = k.model else { continue };
        let facts = facts_of(&ctx, &p.select, &a, model.clone());
        lines.push(status_line(a.name, a.state, &facts, &model));
        *by_state.entry(a.state.to_string()).or_default() += 1;
        *by_model.entry(key_of("model", a.state, &model, &a, &facts, &host)).or_default() += 1;
        if let Some(g) = p.group_by.as_deref() {
            groups.entry(key_of(g, a.state, &model, &a, &facts, &host)).or_default().push(a.name.to_string());
        }
    }
    let mut out = json!({
        "host": host,
        "total": total,
        "returned": lines.len(),
        "lines": lines,
        "counts": { "by_state": by_state, "by_model": by_model, "by_host": { host.clone(): lines.len() } },
    });
    if let Some(g) = p.group_by.as_deref() {
        out["group_by"] = json!(g);
        out["groups"] = json!(groups.into_iter().map(|(k, v)| json!({ "key": k, "count": v.len(), "agents": v })).collect::<Vec<_>>());
    }
    out
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
