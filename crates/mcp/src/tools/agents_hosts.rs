// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents list` and `Agents status` across machines (docs/specs/SPEC_MUXAPI_AGENT_LIVE_FACTS_2026_11_20.md §6):
//! `host` names one LAN peer, or `all` for this machine and every pinned peer. Each peer is asked `agents.status`
//! at once with a short wait; a peer that does not answer, or is too old to know the op, is listed by what the LAN
//! poll already says of it (`facts: "liveness_only"`) and named in the `hosts` block. Nothing is guessed: a missing
//! field is absent.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use futures_util::future::join_all;

use super::*;

/// How long a peer has to answer before it is listed as unreachable.
const PEER_WAIT: Duration = Duration::from_secs(3);

/// What a peer said to `agents.status`, or why it did not.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PeerReply {
    pub host: String,
    /// `ok`, `unreachable`, `needs_upgrade` or `refused`.
    pub state: &'static str,
    pub detail: Option<String>,
    pub version: Option<String>,
    pub as_of_ms: Option<u64>,
    /// The peer's answer: `{agents, page}` for a list, the compact status for a status.
    pub answer: Option<Value>,
}

impl PeerReply {
    fn failed(host: &str, state: &'static str, detail: String) -> Self {
        Self { host: host.to_string(), state, detail: Some(detail), version: None, as_of_ms: None, answer: None }
    }
}

/// The name a peer is asked by: `name`, or `name/channel[/install]` so two instances of one machine are told apart.
fn target_of(peer: &Value) -> String {
    let text = |key: &str| peer.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let (name, channel, install) = (text("hostname"), text("channel"), text("install"));
    match (channel.is_empty(), install.is_empty()) {
        (true, _) => name,
        (false, true) => format!("{name}/{channel}"),
        (false, false) => format!("{name}/{channel}/{install}"),
    }
}

/// The peers `host: all` asks, from the local answer's `peers`: one per instance, with what the LAN poll says of each.
fn peers_of(local: &Value) -> Vec<(String, Value)> {
    let mut seen = HashSet::new();
    local
        .get("peers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| {
            let id = p.get("instance_id").and_then(Value::as_str).filter(|i| !i.is_empty()).map_or_else(|| target_of(p), str::to_string);
            !target_of(p).is_empty() && seen.insert(id)
        })
        .map(|p| (target_of(p), p.clone()))
        .collect()
}

/// How a peer's answer to the op reads: its facts, or the state it is in and why.
fn reply_of(host: &str, answer: &Value) -> PeerReply {
    if answer["status"].as_str() == Some("done") {
        let result = &answer["result"];
        return PeerReply {
            host: host.to_string(),
            state: "ok",
            detail: None,
            version: result["version"].as_str().map(str::to_string),
            as_of_ms: result["as_of_ms"].as_u64(),
            answer: Some(result["answer"].clone()),
        };
    }
    let problem = remote_op::explain_not_done(host, "agents.status", answer).unwrap_or_else(|| format!("{host} did not run agents.status"));
    let state = match (answer["status"].as_str(), answer["refused_reason"].as_str()) {
        (Some("refused"), Some("unknown_op" | "peer_cannot_do_remote_ops")) => "needs_upgrade",
        (Some("refused"), _) => "refused",
        _ => "unreachable",
    };
    PeerReply::failed(host, state, problem)
}

async fn ask(cx: &ToolCtx<'_>, host: &str, params: &Value, reason: &str) -> PeerReply {
    match tokio::time::timeout(PEER_WAIT, remote_op::remote_op_call(cx, host, "agents.status", params.clone(), reason)).await {
        Err(_) => PeerReply::failed(host, "unreachable", format!("{host} did not answer in {} seconds", PEER_WAIT.as_secs())),
        Ok(Err(e)) => PeerReply::failed(host, "unreachable", format!("{e:#}")),
        Ok(Ok(answer)) => reply_of(host, &answer),
    }
}

/// `Agents list` or `Agents status` with `host`: one peer, or `all`.
pub(super) async fn across(cx: &ToolCtx<'_>, args: &Value, status: bool, host: &str) -> Result<String> {
    let (local, targets) = if host.eq_ignore_ascii_case("all") {
        let local = fleet::discover_value(args, status, true, cx).await?;
        let targets = peers_of(&local);
        (Some(local), targets)
    } else {
        (None, vec![(host.to_string(), Value::Null)])
    };
    let params = Value::Object(fleet::discovery_query(args, status).into_iter().map(|(k, v)| (k.to_string(), Value::String(v))).collect());
    let reason = args.get("reason").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("Agents {}", if status { "status" } else { "list" }));
    let replies = join_all(targets.iter().map(|(name, _)| ask(cx, name, &params, &reason))).await;
    let peers: Vec<(PeerReply, Value)> = replies.into_iter().zip(targets.into_iter().map(|(_, lan)| lan)).collect();
    let now_ms = agentmux_common::time::now_ms_u64();
    let merged = if status { merge_status(local, &peers, now_ms) } else { merge_list(local, &peers, now_ms) };
    Ok(serde_json::to_string_pretty(&merged).unwrap_or_else(|_| merged.to_string()))
}

fn tagged(mut row: Value, host: &str) -> Value {
    if let Some(fields) = row.as_object_mut() {
        fields.insert("host".to_string(), json!(host));
    }
    row
}

/// A peer's agents as the LAN poll lists them: name, state, provider and kind, and nothing else.
fn liveness_rows(host: &str, lan: &Value) -> Vec<Value> {
    lan.get("agents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|name| {
            let key = name.to_lowercase();
            let mut row = json!({ "name": name, "host": host, "facts": "liveness_only" });
            for (field, from) in [("state", "agent_status"), ("provider", "agent_providers"), ("kind", "agent_kinds")] {
                if let Some(value) = lan.get(from).and_then(|m| m.get(key.as_str())).filter(|v| !v.is_null()) {
                    row[field] = value.clone();
                }
            }
            row
        })
        .collect()
}

/// One entry of the `hosts` block.
fn host_entry(reply: &PeerReply, lan: &Value, now_ms: u64) -> Value {
    let version = reply.version.clone().or_else(|| lan.get("version").and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string));
    let mut entry = json!({ "host": reply.host, "reachable": reply.state == "ok", "state": reply.state });
    if let Some(v) = version {
        entry["version"] = json!(v);
    }
    if let Some(at) = reply.as_of_ms {
        entry["as_of_ms"] = json!(at);
        entry["age_ms"] = json!(now_ms.saturating_sub(at));
    }
    if let Some(why) = &reply.detail {
        entry["error"] = json!(why);
    }
    entry
}

/// This machine's own `hosts` entry.
fn this_host_entry(local: &Value, now_ms: u64) -> Option<(String, Value)> {
    let name = local.pointer("/host/hostname").or_else(|| local.get("host")).and_then(Value::as_str)?.to_string();
    let version = local.pointer("/host/version").cloned();
    let mut entry = json!({ "host": name, "reachable": true, "state": "ok", "this_machine": true, "as_of_ms": now_ms, "age_ms": 0 });
    if let Some(v) = version.filter(|v| v.is_string()) {
        entry["version"] = v;
    }
    Some((name, entry))
}

/// `Agents list` across machines: every machine's rows, each saying which machine it came from, and the `hosts` block.
pub(super) fn merge_list(local: Option<Value>, peers: &[(PeerReply, Value)], now_ms: u64) -> Value {
    let mut body = local.unwrap_or_else(|| json!({ "host": {} }));
    let (mut hosts, mut rows) = (Vec::new(), Vec::new());
    let mut total = 0u64;
    if let Some((name, entry)) = this_host_entry(&body, now_ms) {
        let own = body.pointer("/host/agents").and_then(Value::as_array).cloned().unwrap_or_default();
        total += body.pointer("/host/page/total").and_then(Value::as_u64).unwrap_or(own.len() as u64);
        rows.extend(own.into_iter().map(|r| tagged(r, &name)));
        hosts.push(entry);
    }
    for (reply, lan) in peers {
        match reply.answer.as_ref().filter(|_| reply.state == "ok") {
            Some(answer) => {
                let theirs = answer.get("agents").and_then(Value::as_array).cloned().unwrap_or_default();
                total += answer.pointer("/page/total").and_then(Value::as_u64).unwrap_or(theirs.len() as u64);
                rows.extend(theirs.into_iter().map(|r| tagged(r, &reply.host)));
            }
            None => {
                let listed = liveness_rows(&reply.host, lan);
                total += listed.len() as u64;
                rows.extend(listed);
            }
        }
        hosts.push(host_entry(reply, lan, now_ms));
    }
    body["host"]["page"] = json!({ "total": total, "returned": rows.len(), "note": "limit and offset apply to each machine" });
    body["host"]["agents"] = Value::Array(rows);
    body["hosts"] = Value::Array(hosts);
    if let Some(fields) = body.as_object_mut() {
        fields.remove("peers");
    }
    body
}

/// A status line with its machine as the second column: `name | host | state ...`.
fn line_with_host(line: &str, host: &str) -> String {
    match line.split_once(" | ") {
        Some((name, rest)) => format!("{name} | {host} | {rest}"),
        None => format!("{line} | {host}"),
    }
}

fn add_counts(into: &mut BTreeMap<String, u64>, counts: Option<&Value>) {
    for (key, n) in counts.and_then(Value::as_object).into_iter().flatten() {
        *into.entry(key.clone()).or_default() += n.as_u64().unwrap_or(0);
    }
}

/// What `Agents status` adds up over the machines.
#[derive(Default)]
struct Tally {
    lines: Vec<String>,
    by_state: BTreeMap<String, u64>,
    by_model: BTreeMap<String, u64>,
    by_host: BTreeMap<String, u64>,
    groups: BTreeMap<String, Vec<String>>,
    total: u64,
    group_by: Option<String>,
}

impl Tally {
    /// A machine's own compact status.
    fn take(&mut self, host: &str, status: &Value) {
        let theirs: Vec<&str> = status.get("lines").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        self.total += status.get("total").and_then(Value::as_u64).unwrap_or(theirs.len() as u64);
        self.by_host.insert(host.to_string(), theirs.len() as u64);
        self.lines.extend(theirs.iter().map(|l| line_with_host(l, host)));
        add_counts(&mut self.by_state, status.pointer("/counts/by_state"));
        add_counts(&mut self.by_model, status.pointer("/counts/by_model"));
        for group in status.get("groups").and_then(Value::as_array).into_iter().flatten() {
            let key = group.get("key").and_then(Value::as_str).unwrap_or("unknown").to_string();
            let members = group.get("agents").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
            self.groups.entry(key).or_default().extend(members.map(|m| format!("{m}@{host}")));
        }
        if self.group_by.is_none() {
            self.group_by = status.get("group_by").and_then(Value::as_str).map(str::to_string);
        }
    }

    /// A machine that gave no facts: its agents as the LAN poll lists them.
    fn liveness(&mut self, host: &str, lan: &Value) {
        let listed = liveness_rows(host, lan);
        self.total += listed.len() as u64;
        self.by_host.insert(host.to_string(), listed.len() as u64);
        self.lines.extend(listed.iter().map(|r| {
            let state = r.get("state").map_or_else(|| "state unknown".to_string(), |s| s.as_str().map_or_else(|| s.to_string(), str::to_string));
            format!("{} | {host} | {state} (liveness only)", r["name"].as_str().unwrap_or_default())
        }));
    }
}

/// `Agents status` across machines: the lines with their machine, the counts added up (and by machine), the groups merged.
pub(super) fn merge_status(local: Option<Value>, peers: &[(PeerReply, Value)], now_ms: u64) -> Value {
    let (mut tally, mut hosts) = (Tally::default(), Vec::new());
    if let Some(local) = &local {
        if let Some((name, entry)) = this_host_entry(local, now_ms) {
            tally.take(&name, local);
            hosts.push(entry);
        }
    }
    for (reply, lan) in peers {
        match reply.answer.as_ref().filter(|_| reply.state == "ok") {
            Some(status) => tally.take(&reply.host, status),
            None => tally.liveness(&reply.host, lan),
        }
        hosts.push(host_entry(reply, lan, now_ms));
    }
    let mut out = json!({
        "total": tally.total,
        "returned": tally.lines.len(),
        "lines": tally.lines,
        "counts": { "by_state": tally.by_state, "by_model": tally.by_model, "by_host": tally.by_host },
        "hosts": hosts,
    });
    if let Some(g) = tally.group_by {
        out["group_by"] = json!(g);
        out["groups"] = json!(tally.groups.into_iter().map(|(k, v)| json!({ "key": k, "count": v.len(), "agents": v })).collect::<Vec<_>>());
    }
    out
}

#[cfg(test)]
#[path = "agents_hosts_tests.rs"]
mod tests;
