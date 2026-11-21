// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! App query block: reads AgentMux's own state, read-only (the Drone real-world jobs spec §4.2).
//! Deterministic and free: no agent turn, no network.
//!
//! Block config (`node.data`):
//!   * `query`   required, one of:
//!       - `muxbus`  whether cloud messages are reaching this machine.
//!       - `agents`  every agent this machine has: how many it wants running, and which of them are not.
//!       - `agent`   one agent's availability. Needs `agent`, its name, written in the graph.
//!       - `tasks`   Kanban tasks, filtered by `holder`, `state` and `pr` (each optional).
//!       - `lan_peers`  the other machines this one sees on the LAN, and whether each answered lately.
//!       - `jobs`    recent job runs: one job's with `job`, else the newest run of every job.
//!       - `memory`  one Global Memory entry, by id or name (`entry`), read-only. Needs `entry`, written in
//!                   the graph. An entry of the system tier is not offered. Nothing here writes Memory.
//!       - `presence`  which agents on this machine are working on a repository, a path or a branch
//!                   (`repo`, `path`, `branch`; at least one), from their own work facts. Names, state and how
//!                   each matched; no goals, no file names, and claims and posts are not read.
//!   * `agent`, `job`, `entry`   the agent's, the job's or the Memory entry's name (a job's or an entry's id
//!               works too). Never a `{{reference}}`: a value from outside must not pick whose state the
//!               graph reads (§6.2).
//!   * `holder`, `state`, `pr`   task filters. They only find tasks, so each may be built from a
//!               `{{reference}}` (a PR url from the trigger). `pr` matches a task's PR link exactly.
//!   * `repo`, `path`, `branch`   the `presence` filters. They only find who is working there, so each may be
//!               built from a `{{reference}}`.
//!   * `max_chars`   `memory`, how much of the entry to return (default 4000, at most 20000).
//!   * `limit`   tasks and jobs, how many to return (tasks: default 50, at most 200; jobs: default 20, at most 100).
//!
//! Output, by query:
//!   * `muxbus`: `{ "state", "connected", "signed_in", "since_ms", "last_ok_ms"?, "last_error"? }`.
//!   * `agents`: `{ "count": N, "wanted_running": W, "up": U, "all_up": bool, "problem_count": P, "problems": [{ "agent",
//!               "state", "detail"? }], "problem_names": "<name (state), ...>" }`. An agent is `up` when it is wanted
//!               running and idle, working or waiting on the person; a wanted-running agent that is stopped, starting,
//!               limited, signed out or in error is a problem (at most 20 listed). `all_up` is true with no problem.
//!   * `agent`:  `{ "found": bool, "ambiguous"?: true, "state"?, "would"?, "reason"?, ... }`, the
//!               availability the Agents tool shows. `found` is false for a name no agent here has.
//!   * `tasks`:  `{ "count": N, "tasks": [{ "num", "title", "state", "holder", "pr", "branch" }] }`.
//!   * `lan_peers`: `{ "count": N, "answered": M, "peers": [{ "host", "channel", "version", "os", "agents",
//!               "last_seen_ms", "last_answered_ms"?, "answered" }] }`. `answered` is true when the peer
//!               answered this machine's own poll (asked every 30 s) in the last two minutes: it is up and
//!               reachable from here. It is not a signature check.
//!   * `jobs`:   (scheduled, Drone and cloud jobs alike, as the run ledger holds them) `{ "found": bool, "count": N, "runs": [{ "id", "job_id", "job", "state", "outcome", "trigger",
//!               "summary", "started_ms", "ended_ms"?, "cost_usd"? }], "last_ok"? }`; `found` is false for a `job` that
//!               matches none. With a `job`, `last_ok` is that job's newest run that finished with outcome `ok` or
//!               `findings` (a run row, or null when there is none), found whatever `limit` cuts `runs` to.
//!   * `memory`: `{ "found": bool, "id"?, "name"?, "content"?, "chars"?, "truncated"? }`; `found` is false for
//!               an entry nobody has (or the system tier), with `ambiguous` when two entries share the name.
//!   * `presence`: `{ "count": N, "agents": [{ "agent", "status", "repo"?, "branch"?, "matched": ["claim", "dirty", ...] }] }`.
//!
//! Every answer of this block is outside text to the run (the taint bits): what an entry or an agent's
//! branch says is not the graph's own.

use serde_json::{json, Value};

use crate::backend::rpc_types::{MuxBusDeliveryState, MuxBusDeliveryStatus};
use crate::backend::storage::work_board::BoardTask;
use crate::drone::data_flow::ExecutionScope;
use crate::drone::types::FlowNode;

/// Tasks returned when the node gives no `limit`.
const DEFAULT_LIMIT: usize = 50;
/// The most tasks one query returns.
const MAX_LIMIT: usize = 200;
/// How many board tasks are read before filtering: every open task and the recently finished ones.
const SCAN_LIMIT: usize = 2_000;
/// Finished tasks changed within this many days are still seen by a query.
const FINISHED_WINDOW_DAYS: i64 = 30;
/// Job runs returned when the node gives no `limit`, and the most one query returns.
const DEFAULT_RUNS: usize = 20;
const MAX_RUNS: usize = 100;
/// A peer that answered this machine's poll within this many seconds counts as answering
/// (the poll is every 30 s, so four missed ones in a row).
const ANSWERED_WITHIN_SECS: u64 = 120;
/// A run's summary is cut to this many characters.
const RUN_SUMMARY_CHARS: usize = 200;
/// A Memory entry is cut to this many characters when the node gives no `max_chars`, and never to more than the cap.
const DEFAULT_MEMORY_CHARS: usize = 4_000;
const MAX_MEMORY_CHARS: usize = 20_000;
/// The most agents a `presence` query lists.
const MAX_PRESENCE: usize = 50;

/// What the block asks.
#[derive(Debug, PartialEq)]
pub(crate) enum Query {
    MuxBus,
    Agents,
    Agent(String),
    Tasks(TaskFilter),
    LanPeers,
    Jobs(JobsFilter),
    Memory(MemoryFilter),
    Presence(PresenceFilter),
}

/// The filters of a `memory` query: one Global Memory entry, by id or name, and how much of it to return.
#[derive(Debug, PartialEq, Default)]
pub(crate) struct MemoryFilter {
    pub entry: String,
    pub max_chars: usize,
}

/// The filters of a `presence` query: what to ask who is working on; at least one is not empty.
#[derive(Debug, PartialEq, Default)]
pub(crate) struct PresenceFilter {
    pub repo: String,
    pub path: String,
    pub branch: String,
}

/// The filters of a `jobs` query: one job by name or id (empty is every job) and how many runs.
#[derive(Debug, PartialEq, Default)]
pub(crate) struct JobsFilter {
    pub job: String,
    pub limit: usize,
}

/// The filters of a `tasks` query; an empty one matches anything.
#[derive(Debug, PartialEq, Default)]
pub(crate) struct TaskFilter {
    pub holder: String,
    pub state: String,
    pub pr: String,
    pub limit: usize,
}

pub async fn run(node: &FlowNode, scope: &ExecutionScope) -> Result<Value, String> {
    match plan(node, scope)? {
        Query::MuxBus => Ok(muxbus_answer(&crate::muxbus::delivery_status::status())),
        Query::Agents => {
            let state = crate::drone::host::state()?.clone();
            let now = agentmux_common::time::now_ms_u64();
            tokio::task::spawn_blocking(move || agents_answer(&crate::server::agent_runtime::list(&state, now)))
                .await
                .map_err(|e| format!("app query: {e}"))
        }
        Query::Agent(name) => {
            let state = crate::drone::host::state()?;
            Ok(agent_answer(crate::server::availability_view::view_of_agent(state, &name).await))
        }
        Query::LanPeers => {
            let state = crate::drone::host::state()?;
            let own = crate::backend::lan::listeners::cached_local_addresses();
            Ok(lan_peers_answer(&state.lan_discovery.get_instances(), &own, agentmux_common::time::now_secs() as u64))
        }
        Query::Jobs(filter) => {
            let state = crate::drone::host::state()?;
            let (Some(jobs), Some(ledger)) = (crate::server::cron::jobs_store(state), state.cron_scheduler.runs().cloned()) else {
                return Err("app query: the job ledger isn't ready yet".to_string());
            };
            tokio::task::spawn_blocking(move || jobs_answer(&jobs, ledger.store(), &filter))
                .await
                .map_err(|e| format!("app query: {e}"))?
        }
        Query::Memory(filter) => {
            let state = crate::drone::host::state()?.clone();
            tokio::task::spawn_blocking(move || {
                let entries = state.id_store.bundle_list_global().map_err(|e| format!("app query: couldn't read Global Memory: {e}"))?;
                Ok(memory_answer(&entries, &filter))
            })
            .await
            .map_err(|e| format!("app query: {e}"))?
        }
        Query::Presence(filter) => {
            let state = crate::drone::host::state()?;
            let facts = crate::server::work_facts_handlers::local_facts(state).await;
            presence_answer(&facts, &filter)
        }
        Query::Tasks(filter) => {
            let state = crate::drone::host::state()?;
            let store = crate::server::cron::work_store(state);
            let now = agentmux_common::time::now_ms() as i64;
            let since = now - FINISHED_WINDOW_DAYS * 24 * 3_600_000;
            let all = tokio::task::spawn_blocking(move || store.work_board_list(since, SCAN_LIMIT, now))
                .await
                .map_err(|e| format!("app query: {e}"))?
                .map_err(|e| format!("app query: couldn't read the tasks: {e}"))?;
            Ok(tasks_answer(&all, &filter))
        }
    }
}

/// The question a `poll` trigger asks (`backend::jobs::poll`): the App query named by the filter's `query`, with
/// the filter's own keys as its config and no references to resolve.
pub(crate) fn poll_check() -> crate::backend::jobs::poll::Check {
    std::sync::Arc::new(|filter| {
        Box::pin(async move {
            // A poll of a site outside AgentMux: the API block's own call, which never asks the person (`api_poll`).
            if filter.get("query").and_then(|q| q.as_str()) == Some("api") {
                return super::api_poll::ask(filter).await;
            }
            let node = FlowNode { id: "poll".to_string(), position: Default::default(), data: Value::Object(filter), node_type: None };
            run(&node, &ExecutionScope::new()).await
        })
    })
}

/// What the node asks, with the filters' references resolved. Pure, so it is testable without a server.
pub(crate) fn plan(node: &FlowNode, scope: &ExecutionScope) -> Result<Query, String> {
    let text = |key: &str| node.data.get(key).and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
    match text("query") {
        "muxbus" => Ok(Query::MuxBus),
        "agents" => Ok(Query::Agents),
        "agent" => {
            let name = text("agent");
            if name.is_empty() {
                return Err("app query: the `agent` query needs `agent`".to_string());
            }
            if name.contains("{{") {
                return Err("app query: `agent` must name an agent, not a {{reference}}".to_string());
            }
            Ok(Query::Agent(name.to_string()))
        }
        "tasks" => {
            let limit = match node.data.get("limit").and_then(|v| v.as_u64()) {
                Some(n) => (n as usize).clamp(1, MAX_LIMIT),
                None => DEFAULT_LIMIT,
            };
            let resolved = |key: &str| scope.resolve(text(key)).trim().to_string();
            Ok(Query::Tasks(TaskFilter { holder: resolved("holder"), state: resolved("state"), pr: resolved("pr"), limit }))
        }
        "lan_peers" => Ok(Query::LanPeers),
        "jobs" => {
            let job = text("job");
            if job.contains("{{") {
                return Err("app query: `job` must name a job, not a {{reference}}".to_string());
            }
            let limit = match node.data.get("limit").and_then(|v| v.as_u64()) {
                Some(n) => (n as usize).clamp(1, MAX_RUNS),
                None => DEFAULT_RUNS,
            };
            Ok(Query::Jobs(JobsFilter { job: job.to_string(), limit }))
        }
        "memory" => {
            let entry = text("entry");
            if entry.is_empty() {
                return Err("app query: the `memory` query needs `entry`".to_string());
            }
            if entry.contains("{{") {
                return Err("app query: `entry` must name a Global Memory entry, not a {{reference}}".to_string());
            }
            let max_chars = match node.data.get("max_chars").and_then(|v| v.as_u64()) {
                Some(n) => (n as usize).clamp(1, MAX_MEMORY_CHARS),
                None => DEFAULT_MEMORY_CHARS,
            };
            Ok(Query::Memory(MemoryFilter { entry: entry.to_string(), max_chars }))
        }
        "presence" => {
            let resolved = |key: &str| scope.resolve(text(key)).trim().to_string();
            let filter = PresenceFilter { repo: resolved("repo"), path: resolved("path"), branch: resolved("branch") };
            if filter.repo.is_empty() && filter.path.is_empty() && filter.branch.is_empty() {
                return Err("app query: the `presence` query needs `repo`, `path` or `branch`".to_string());
            }
            Ok(Query::Presence(filter))
        }
        "" => Err("app query block missing `query`".to_string()),
        other => Err(format!("app query: unknown query {other:?}; this AgentMux has muxbus, agents, agent, tasks, lan_peers, jobs, memory and presence")),
    }
}

/// `muxbus`: the delivery state in the words a Condition reads.
pub(crate) fn muxbus_answer(s: &MuxBusDeliveryStatus) -> Value {
    let mut out = json!({
        "state": serde_json::to_value(s.state).unwrap_or(Value::Null),
        "connected": s.state == MuxBusDeliveryState::Connected,
        "signed_in": matches!(s.state, MuxBusDeliveryState::Connected | MuxBusDeliveryState::Reconnecting),
        "since_ms": s.since_ms,
    });
    if let Some(ms) = s.last_ok_ms {
        out["last_ok_ms"] = json!(ms);
    }
    if let Some(e) = &s.last_error {
        out["last_error"] = json!(e);
    }
    out
}

/// `agent`: the availability view, with `found` first.
fn agent_answer(known: crate::server::availability_view::Known) -> Value {
    use crate::server::availability_view::Known;
    match known {
        Known::Live(mut v) => {
            if let Some(o) = v.as_object_mut() {
                o.insert("found".to_string(), json!(true));
            }
            v
        }
        Known::Ambiguous => json!({ "found": false, "ambiguous": true }),
        Known::Unknown => json!({ "found": false }),
    }
}

/// The most agents `agents` lists as a problem.
const MAX_PROBLEMS: usize = 20;

/// `agents`: how many agents the machine wants running and which of those are not up. Pure, so a test can hand it rows.
pub(crate) fn agents_answer(rows: &[crate::backend::agent_runtime_state::AgentRuntime]) -> Value {
    use crate::backend::agent_runtime_state::{AgentDesiredState, AgentRuntimeState as S};
    let wanted: Vec<_> = rows.iter().filter(|r| r.desired == AgentDesiredState::Running).collect();
    let is_up = |r: &crate::backend::agent_runtime_state::AgentRuntime| matches!(r.state, S::Idle | S::Working | S::WaitingOnUser);
    let up = wanted.iter().filter(|r| is_up(r)).count();
    let down: Vec<_> = wanted.iter().filter(|r| !is_up(r)).collect();
    let state_name = |r: &crate::backend::agent_runtime_state::AgentRuntime| serde_json::to_value(r.state).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let problems: Vec<Value> = down
        .iter()
        .take(MAX_PROBLEMS)
        .map(|r| {
            let mut p = json!({ "agent": r.name, "state": state_name(r) });
            if let Some(d) = &r.detail {
                p["detail"] = json!(d);
            }
            p
        })
        .collect();
    let names = down.iter().take(MAX_PROBLEMS).map(|r| format!("{} ({})", r.name, state_name(r))).collect::<Vec<_>>().join(", ");
    json!({ "count": rows.len(), "wanted_running": wanted.len(), "up": up, "all_up": down.is_empty(), "problem_count": down.len(), "problems": problems, "problem_names": names })
}

/// `tasks`: the tasks that match, newest first as the board lists them, cut to the limit.
pub(crate) fn tasks_answer(all: &[BoardTask], f: &TaskFilter) -> Value {
    let hits: Vec<Value> = all
        .iter()
        .filter(|t| f.holder.is_empty() || t.item.claimed_by == f.holder)
        .filter(|t| f.state.is_empty() || t.item.state == f.state)
        .filter(|t| f.pr.is_empty() || t.card.pr_url == f.pr)
        .take(f.limit)
        .map(|t| {
            json!({
                "num": t.card.num,
                "title": t.item.title,
                "state": t.item.state,
                "holder": t.item.claimed_by,
                "pr": t.card.pr_url,
                "branch": t.card.branch,
            })
        })
        .collect();
    json!({ "count": hits.len(), "tasks": hits })
}

/// `lan_peers`: the machines on the LAN other than this one (a peer at one of this machine's own addresses
/// is another channel here), by host name, each with whether it answered the poll lately.
pub(crate) fn lan_peers_answer(peers: &[crate::backend::lan::discovery::LanInstance], own_addrs: &std::collections::HashSet<std::net::IpAddr>, now_secs: u64) -> Value {
    let mut rows: Vec<(String, Value, bool)> = peers
        .iter()
        .filter(|p| !p.address.parse::<std::net::IpAddr>().is_ok_and(|a| own_addrs.contains(&a)))
        .map(|p| {
            let answered = p.last_polled_ok > 0 && now_secs.saturating_sub(p.last_polled_ok) <= ANSWERED_WITHIN_SECS;
            let host = if p.hostname.trim().is_empty() { "unnamed host".to_string() } else { p.hostname.trim().to_string() };
            let mut row = json!({
                "host": host,
                "channel": p.channel,
                "version": p.version,
                "os": p.os,
                "agents": p.agents.len(),
                "last_seen_ms": p.last_seen.saturating_mul(1000),
                "answered": answered,
            });
            if p.last_polled_ok > 0 {
                row["last_answered_ms"] = json!(p.last_polled_ok.saturating_mul(1000));
            }
            (host.to_lowercase(), row, answered)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let answered = rows.iter().filter(|r| r.2).count();
    json!({ "count": rows.len(), "answered": answered, "peers": rows.into_iter().map(|r| r.1).collect::<Vec<_>>() })
}

/// One run, for the graph: what happened and when, with the summary cut.
fn run_row(r: &crate::backend::storage::job_runs::JobRun) -> Value {
    let mut row = json!({
        "id": r.id,
        "job_id": r.job_id,
        "job": r.job_name,
        "state": r.state,
        "outcome": r.outcome,
        "trigger": r.trigger,
        "summary": r.summary.chars().take(RUN_SUMMARY_CHARS).collect::<String>(),
        "started_ms": r.started_at_ms,
    });
    if let Some(end) = r.ended_at_ms {
        row["ended_ms"] = json!(end);
    }
    if r.cost_known {
        row["cost_usd"] = json!(r.cost_usd);
    }
    row
}

/// `jobs`: the named job's newest runs, or the newest run of every job, newest first, cut to the limit. A job is
/// whatever the run ledger has runs of (scheduled, Drone and cloud jobs alike), and a scheduled job with no run yet.
pub(crate) fn jobs_answer(jobs: &crate::backend::storage::store::Store, ledger: &crate::backend::storage::store::Store, f: &JobsFilter) -> Result<Value, String> {
    let read = |e: crate::backend::storage::StoreError| format!("app query: couldn't read the runs: {e}");
    if f.job.is_empty() {
        let rows: Vec<Value> = ledger.job_runs_latest_per_job(f.limit as i64).map_err(read)?.iter().map(run_row).collect();
        return Ok(json!({ "found": true, "count": rows.len(), "runs": rows }));
    }
    let mut ids = ledger.job_ids_named(&f.job).map_err(read)?;
    for job in jobs.cron_list().map_err(|e| format!("app query: couldn't read the jobs: {e}"))? {
        if (job.id == f.job || job.name == f.job) && !ids.contains(&job.id) {
            ids.push(job.id);
        }
    }
    match ids.as_slice() {
        [] => Ok(json!({ "found": false, "count": 0, "runs": [] })),
        [id] => {
            let rows: Vec<Value> = ledger.job_runs_for_job(id, f.limit as i64, None).map_err(read)?.iter().map(run_row).collect();
            let last_ok = ledger.job_run_last_good(id).map_err(read)?.map(|r| run_row(&r)).unwrap_or(Value::Null);
            Ok(json!({ "found": true, "count": rows.len(), "runs": rows, "last_ok": last_ok }))
        }
        _ => Ok(json!({ "found": false, "ambiguous": true, "count": 0, "runs": [] })),
    }
}

/// `memory`: the Global Memory entry whose id is `f.entry`, or whose name is (without regard to case), cut to
/// `f.max_chars`. An entry of the system tier is not offered, and two entries with one name are not guessed between.
pub(crate) fn memory_answer(entries: &[crate::backend::storage::bundles::Bundle], f: &MemoryFilter) -> Value {
    let want = f.entry.to_lowercase();
    let mut hits = entries.iter().filter(|b| b.is_global && !b.is_system && (b.id == f.entry || b.name.to_lowercase() == want));
    match (hits.next(), hits.next()) {
        (Some(b), None) => {
            let chars = b.instructions.chars().count();
            let content: String = b.instructions.chars().take(f.max_chars).collect();
            json!({ "found": true, "id": b.id, "name": b.name, "content": content, "chars": chars, "truncated": chars > f.max_chars })
        }
        (Some(_), Some(_)) => json!({ "found": false, "ambiguous": true }),
        _ => json!({ "found": false }),
    }
}

/// `presence`: the agents whose own work facts (their clones' uncommitted changes, recent edits, branch and repository)
/// touch what is asked. Claims and posts are not read. Only what `Presence who` shows of each agent that a graph needs:
/// its name, its state and how it matched; no goal, no file names. Pure over the facts, so a test can hand it some.
pub(crate) fn presence_answer(facts: &[crate::backend::work_facts::WorkFacts], f: &PresenceFilter) -> Result<Value, String> {
    use crate::backend::work_facts::matching;
    let asked = matching::WhoQuery { path: Some(f.path.clone()), repo: Some(f.repo.clone()), branch: Some(f.branch.clone()), query: None };
    let target = matching::resolve(&asked, None, facts).map_err(|e| format!("app query: {e}"))?;
    let found = matching::who_is_working_on(&target, facts);
    let agents: Vec<Value> = found
        .iter()
        .take(MAX_PRESENCE)
        .map(|r| {
            let matched: Vec<Value> = r.matches.iter().map(|m| serde_json::to_value(m.kind).unwrap_or(Value::Null)).collect();
            let mut row = json!({ "agent": r.agent, "status": r.status, "matched": matched });
            if let Some(repo) = &r.repo {
                row["repo"] = json!(repo);
            }
            if let Some(branch) = &r.branch {
                row["branch"] = json!(branch);
            }
            row
        })
        .collect();
    Ok(json!({ "count": found.len(), "agents": agents }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::work_board::WorkCard;
    use crate::backend::storage::work_queue::WorkItem;
    use crate::drone::types::NodePosition;

    fn node(data: Value) -> FlowNode {
        FlowNode { id: "q1".to_string(), position: NodePosition::default(), data, node_type: None }
    }

    fn task(num: i64, state: &str, holder: &str, pr: &str) -> BoardTask {
        let item: WorkItem = serde_json::from_value(json!({
            "id": format!("w{num}"), "title": format!("task {num}"), "payload": "", "state": state,
            "claimed_by": holder, "created_at": 0, "updated_at": 0,
        }))
        .unwrap();
        BoardTask { item, card: WorkCard { num, pr_url: pr.to_string(), ..Default::default() } }
    }

    #[test]
    fn the_query_names_what_is_read_and_an_unknown_one_says_what_exists() {
        let s = ExecutionScope::new();
        assert_eq!(plan(&node(json!({ "query": "muxbus" })), &s).unwrap(), Query::MuxBus);
        let err = plan(&node(json!({ "query": "weather" })), &s).unwrap_err();
        assert!(err.contains("muxbus, agents, agent, tasks, lan_peers, jobs, memory and presence"), "{err}");
        assert!(plan(&node(json!({})), &s).is_err());
    }

    #[test]
    fn the_agent_is_named_in_the_graph_and_never_by_a_reference() {
        let s = ExecutionScope::new();
        assert_eq!(plan(&node(json!({ "query": "agent", "agent": " clamk " })), &s).unwrap(), Query::Agent("clamk".to_string()));
        assert!(plan(&node(json!({ "query": "agent" })), &s).is_err());
        let err = plan(&node(json!({ "query": "agent", "agent": "{{trigger.who}}" })), &s).unwrap_err();
        assert!(err.contains("not a {{reference}}"), "{err}");
    }

    #[test]
    fn task_filters_may_come_from_a_reference_and_the_limit_is_capped() {
        let s = ExecutionScope::with_trigger(json!({ "url": "https://github.com/o/r/pull/7" }));
        let q = plan(&node(json!({ "query": "tasks", "pr": "{{trigger.url}}", "state": "claimed", "limit": 9999 })), &s).unwrap();
        assert_eq!(
            q,
            Query::Tasks(TaskFilter { holder: String::new(), state: "claimed".to_string(), pr: "https://github.com/o/r/pull/7".to_string(), limit: MAX_LIMIT })
        );
        let q = plan(&node(json!({ "query": "tasks" })), &s).unwrap();
        assert_eq!(q, Query::Tasks(TaskFilter { limit: DEFAULT_LIMIT, ..Default::default() }));
    }

    fn entry(id: &str, name: &str, text: &str, system: bool) -> crate::backend::storage::bundles::Bundle {
        crate::backend::storage::bundles::Bundle {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            is_blank: false,
            is_global: true,
            provider: String::new(),
            model: String::new(),
            instructions: text.to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            is_system: system,
        }
    }

    #[test]
    fn the_memory_entry_is_named_in_the_graph_and_never_by_a_reference() {
        let s = ExecutionScope::new();
        assert_eq!(
            plan(&node(json!({ "query": "memory", "entry": " Release rules " })), &s).unwrap(),
            Query::Memory(MemoryFilter { entry: "Release rules".to_string(), max_chars: DEFAULT_MEMORY_CHARS })
        );
        assert!(plan(&node(json!({ "query": "memory" })), &s).is_err(), "an entry is required");
        let err = plan(&node(json!({ "query": "memory", "entry": "{{trigger.name}}" })), &s).unwrap_err();
        assert!(err.contains("not a {{reference}}"), "{err}");
        let capped = plan(&node(json!({ "query": "memory", "entry": "x", "max_chars": 999_999 })), &s).unwrap();
        assert_eq!(capped, Query::Memory(MemoryFilter { entry: "x".to_string(), max_chars: MAX_MEMORY_CHARS }));
    }

    #[test]
    fn a_memory_entry_is_read_by_id_or_name_and_cut_to_the_limit() {
        let all = vec![entry("b1", "Release rules", "one two three four", false), entry("b2", "Other", "x", false)];
        let f = |e: &str, n: usize| MemoryFilter { entry: e.to_string(), max_chars: n };
        let by_name = memory_answer(&all, &f("release RULES", 100));
        assert_eq!((by_name["found"].clone(), by_name["id"].clone(), by_name["content"].clone()), (json!(true), json!("b1"), json!("one two three four")));
        assert_eq!(by_name["truncated"], json!(false));
        let cut = memory_answer(&all, &f("b1", 7));
        assert_eq!((cut["content"].clone(), cut["chars"].clone(), cut["truncated"].clone()), (json!("one two"), json!(18), json!(true)));
        assert_eq!(memory_answer(&all, &f("nothing", 10)), json!({ "found": false }));
    }

    #[test]
    fn the_system_tier_and_an_ambiguous_name_are_not_offered() {
        let all = vec![entry("s1", "Baseline", "secret rules", true), entry("b1", "Notes", "a", false), entry("b2", "notes", "b", false)];
        let f = |e: &str| MemoryFilter { entry: e.to_string(), max_chars: 100 };
        assert_eq!(memory_answer(&all, &f("Baseline")), json!({ "found": false }), "a system entry is not found by name");
        assert_eq!(memory_answer(&all, &f("s1")), json!({ "found": false }), "nor by id");
        assert_eq!(memory_answer(&all, &f("Notes")), json!({ "found": false, "ambiguous": true }));
        assert_eq!(memory_answer(&all, &f("b2"))["found"], json!(true), "an id still picks one of two same-named entries");
    }

    #[test]
    fn a_memory_entry_is_cut_on_characters_not_bytes() {
        let all = vec![entry("b1", "N", "h\u{e9}llo w\u{f6}rld", false)];
        let out = memory_answer(&all, &MemoryFilter { entry: "N".to_string(), max_chars: 4 });
        assert_eq!(out["content"], json!("h\u{e9}ll"));
    }

    #[test]
    fn presence_needs_something_to_look_for_and_its_filters_may_be_references() {
        let s = ExecutionScope::with_trigger(json!({ "branch": "loap/fix" }));
        let err = plan(&node(json!({ "query": "presence" })), &s).unwrap_err();
        assert!(err.contains("needs `repo`, `path` or `branch`"), "{err}");
        let blank = ExecutionScope::with_trigger(json!({ "branch": "" }));
        assert!(plan(&node(json!({ "query": "presence", "branch": "{{trigger.branch}}" })), &blank).is_err(), "a reference that is empty is nothing to look for");
        assert_eq!(
            plan(&node(json!({ "query": "presence", "repo": " a/b ", "branch": "{{trigger.branch}}" })), &s).unwrap(),
            Query::Presence(PresenceFilter { repo: "a/b".to_string(), path: String::new(), branch: "loap/fix".to_string() })
        );
    }

    fn facts(agent: &str, repo: &str, branch: &str, status: &str) -> crate::backend::work_facts::WorkFacts {
        crate::backend::work_facts::WorkFacts {
            agent: agent.to_string(),
            repo: Some(repo.to_string()),
            branch: Some(branch.to_string()),
            status: status.to_string(),
            goal: Some("a goal the graph must not see".to_string()),
            dirty_files: vec!["secret/file.rs".to_string()],
            dirty_count: 1,
            ..Default::default()
        }
    }

    #[test]
    fn presence_lists_who_is_on_the_repo_and_branch_without_goals_or_file_names() {
        let all = vec![facts("loap", "a/b", "loap/fix", "busy"), facts("opaz", "a/b", "main", "idle"), facts("other", "c/d", "main", "idle")];
        let f = |repo: &str, branch: &str| PresenceFilter { repo: repo.to_string(), path: String::new(), branch: branch.to_string() };
        let on_repo = presence_answer(&all, &f("a/b", "")).unwrap();
        assert_eq!(on_repo["count"], json!(2));
        let names: Vec<_> = on_repo["agents"].as_array().unwrap().iter().map(|a| a["agent"].as_str().unwrap().to_string()).collect();
        assert!(names.contains(&"loap".to_string()) && names.contains(&"opaz".to_string()) && !names.contains(&"other".to_string()), "{names:?}");
        let text = on_repo.to_string();
        assert!(!text.contains("a goal the graph must not see") && !text.contains("secret/file.rs"), "{text}");
        let on_branch = presence_answer(&all, &f("a/b", "loap/fix")).unwrap();
        assert!(on_branch["agents"].as_array().unwrap().iter().any(|a| a["agent"] == "loap" && a["matched"].as_array().is_some_and(|m| !m.is_empty())));
    }

    #[test]
    fn a_relative_path_with_no_repository_is_an_error_not_an_empty_answer() {
        let all = vec![facts("loap", "a/b", "main", "idle")];
        let f = PresenceFilter { repo: String::new(), path: "src/lib.rs".to_string(), branch: String::new() };
        assert!(presence_answer(&all, &f).unwrap_err().starts_with("app query: "));
    }

    #[test]
    fn tasks_are_found_by_holder_state_and_pr_link() {
        let all = vec![task(1, "claimed", "masty", "https://x/pull/1"), task(2, "done", "masty", "https://x/pull/2"), task(3, "claimed", "maop", "https://x/pull/1")];
        let f = |holder: &str, state: &str, pr: &str| TaskFilter { holder: holder.into(), state: state.into(), pr: pr.into(), limit: 50 };
        assert_eq!(tasks_answer(&all, &f("masty", "", ""))["count"], 2);
        assert_eq!(tasks_answer(&all, &f("", "claimed", "https://x/pull/1"))["count"], 2);
        let one = tasks_answer(&all, &f("masty", "claimed", "https://x/pull/1"));
        assert_eq!(one["count"], 1);
        assert_eq!(one["tasks"][0]["num"], 1);
        assert_eq!(tasks_answer(&all, &f("nobody", "", ""))["count"], 0);
        assert_eq!(tasks_answer(&all, &TaskFilter { limit: 1, ..Default::default() })["count"], 1);
    }

    fn peer(host: &str, address: &str, last_seen: u64, polled: u64) -> crate::backend::lan::discovery::LanInstance {
        serde_json::from_value(json!({
            "instance_id": format!("{host}-i"), "hostname": host, "version": "0.60.26", "channel": "stable", "os": "linux", "install": "", "build": "",
            "address": address, "port": 29700, "agents": ["a", "b"], "agent_status": {}, "agent_lines": {}, "agent_providers": {},
            "first_seen": 1, "last_seen": last_seen, "last_polled_ok": polled, "other_ttl_secs": 4500,
        }))
        .unwrap()
    }

    #[test]
    fn lan_peers_say_who_answered_the_poll_lately_and_leave_out_this_machines_own() {
        let own: std::collections::HashSet<std::net::IpAddr> = ["203.0.113.21".parse().unwrap()].into_iter().collect();
        let now = 10_000;
        let peers = vec![peer("narko", "203.0.113.10", now - 5, now - 30), peer("charlie", "203.0.113.11", now - 5, now - 600), peer("area54", "203.0.113.12", now - 5, 0), peer("starpower-2", "203.0.113.21", now, now)];
        let a = lan_peers_answer(&peers, &own, now);
        assert_eq!((a["count"].clone(), a["answered"].clone()), (json!(3), json!(1)), "this machine's own address is another channel here");
        let hosts: Vec<&str> = a["peers"].as_array().unwrap().iter().map(|p| p["host"].as_str().unwrap()).collect();
        assert_eq!(hosts, vec!["area54", "charlie", "narko"], "by host name");
        let by = |h: &str| a["peers"].as_array().unwrap().iter().find(|p| p["host"] == h).unwrap().clone();
        assert_eq!(by("narko")["answered"], json!(true));
        assert_eq!(by("charlie")["answered"], json!(false), "answered ten minutes ago");
        assert!(by("area54").get("last_answered_ms").is_none() && by("area54")["answered"] == json!(false), "never answered");
        assert_eq!(by("narko")["last_answered_ms"], json!((now - 30) * 1000));
        assert_eq!(by("narko")["agents"], json!(2));
    }

    /// A test state whose store holds the jobs and, through `job_runs_ensure_schema`, their runs.
    fn ledger_with(jobs: &[(&str, &str)], runs: &[(&str, &str, i64)]) -> crate::server::AppState {
        use crate::backend::storage::cron::CronJob;
        use crate::backend::storage::job_runs::JobRun;
        let state = crate::server::tests::test_state();
        state.mstore.job_runs_ensure_schema().unwrap();
        for (id, name) in jobs {
            state.mstore.cron_create(&CronJob { id: id.to_string(), name: name.to_string(), ..Default::default() }).unwrap();
        }
        for (id, job, at) in runs {
            let name = jobs.iter().find(|j| j.0 == *job).map(|j| j.1).unwrap_or("");
            state.mstore.job_run_insert(&JobRun::new(id, job, "cron", name, "schedule", "finished", *at)).unwrap();
        }
        state
    }

    #[tokio::test]
    async fn jobs_without_a_name_give_the_newest_run_of_each_job_newest_first() {
        let state = ledger_with(&[("j1", "Nightly"), ("j2", "Backup")], &[("r1", "j1", 100), ("r2", "j1", 300), ("r3", "j2", 200)]);
        let a = jobs_answer(&state.mstore, &state.mstore, &JobsFilter { job: String::new(), limit: 20 }).unwrap();
        let ids: Vec<&str> = a["runs"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!((a["found"].clone(), a["count"].clone(), ids), (json!(true), json!(2), vec!["r2", "r3"]));
        assert_eq!(jobs_answer(&state.mstore, &state.mstore, &JobsFilter { job: String::new(), limit: 1 }).unwrap()["count"], 1);
    }

    #[tokio::test]
    async fn jobs_of_other_sources_count_too_and_a_scheduled_job_with_no_run_is_found() {
        use crate::backend::storage::job_runs::JobRun;
        let state = ledger_with(&[("j1", "Nightly"), ("j9", "Idle")], &[("r1", "j1", 100)]);
        state.mstore.job_run_insert(&JobRun::new("d1", "drone-1", "drone", "Report drone", "manual", "finished", 500)).unwrap();
        state.mstore.job_run_insert(&JobRun::new("c1", "tpl.cloud", "cloud", "Cloud check", "event", "finished", 400)).unwrap();
        let all = jobs_answer(&state.mstore, &state.mstore, &JobsFilter { job: String::new(), limit: 20 }).unwrap();
        let ids: Vec<&str> = all["runs"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["d1", "c1", "r1"], "the Drone and cloud jobs' newest runs are in, newest first");
        for who in ["Report drone", "drone-1", "Cloud check", "tpl.cloud"] {
            let a = jobs_answer(&state.mstore, &state.mstore, &JobsFilter { job: who.to_string(), limit: 20 }).unwrap();
            assert_eq!((a["found"].clone(), a["count"].clone()), (json!(true), json!(1)), "{who}");
        }
        let idle = jobs_answer(&state.mstore, &state.mstore, &JobsFilter { job: "Idle".to_string(), limit: 20 }).unwrap();
        assert_eq!((idle["found"].clone(), idle["count"].clone()), (json!(true), json!(0)), "a scheduled job that hasn't run yet exists");
    }

    #[tokio::test]
    async fn a_named_job_says_its_last_good_run_even_when_newer_runs_failed_and_the_limit_hides_it() {
        use crate::backend::storage::job_runs::JobRun;
        let state = ledger_with(&[("j1", "Nightly")], &[]);
        let run = |id: &str, outcome: &str, at: i64| {
            let mut r = JobRun::new(id, "j1", "cron", "Nightly", "schedule", "finished", at);
            r.outcome = outcome.to_string();
            r.ended_at_ms = Some(at + 10);
            state.mstore.job_run_insert(&r).unwrap();
        };
        run("old-ok", "ok", 100);
        run("fail-1", "failed", 200);
        run("fail-2", "failed", 300);
        let a = jobs_answer(&state.mstore, &state.mstore, &JobsFilter { job: "Nightly".to_string(), limit: 2 }).unwrap();
        let ids: Vec<&str> = a["runs"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["fail-2", "fail-1"], "the listing is cut to the two newest, both failed");
        assert_eq!(a["last_ok"]["id"], json!("old-ok"), "the last good run is not in it, and is still said");
        assert_eq!(a["last_ok"]["ended_ms"], json!(110));
        let none = ledger_with(&[("j2", "Never")], &[]);
        let n = jobs_answer(&none.mstore, &none.mstore, &JobsFilter { job: "Never".to_string(), limit: 20 }).unwrap();
        assert!(n["last_ok"].is_null(), "no good run: null");
    }

    #[tokio::test]
    async fn a_named_job_gives_its_own_runs_by_name_or_id_and_an_unknown_one_is_not_found() {
        let state = ledger_with(&[("j1", "Nightly"), ("j2", "Backup")], &[("r1", "j1", 100), ("r2", "j1", 300), ("r3", "j2", 200)]);
        for who in ["Nightly", "j1"] {
            let a = jobs_answer(&state.mstore, &state.mstore, &JobsFilter { job: who.to_string(), limit: 20 }).unwrap();
            let ids: Vec<&str> = a["runs"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
            assert_eq!(ids, vec!["r2", "r1"], "{who}: its own runs, newest first");
        }
        let none = jobs_answer(&state.mstore, &state.mstore, &JobsFilter { job: "Nope".to_string(), limit: 20 }).unwrap();
        assert_eq!((none["found"].clone(), none["count"].clone()), (json!(false), json!(0)));
    }

    #[test]
    fn the_job_is_named_in_the_graph_and_never_by_a_reference_and_the_run_limit_is_capped() {
        let s = ExecutionScope::new();
        assert_eq!(plan(&node(json!({ "query": "jobs", "job": " Nightly ", "limit": 9999 })), &s).unwrap(), Query::Jobs(JobsFilter { job: "Nightly".to_string(), limit: MAX_RUNS }));
        assert_eq!(plan(&node(json!({ "query": "jobs" })), &s).unwrap(), Query::Jobs(JobsFilter { job: String::new(), limit: DEFAULT_RUNS }));
        assert!(plan(&node(json!({ "query": "jobs", "job": "{{trigger.job}}" })), &s).unwrap_err().contains("not a {{reference}}"));
        assert_eq!(plan(&node(json!({ "query": "lan_peers" })), &s).unwrap(), Query::LanPeers);
    }

    #[test]
    fn muxbus_says_connected_and_signed_in_in_words_a_condition_can_test() {
        let status = |state| MuxBusDeliveryStatus { state, since_ms: 5, last_ok_ms: None, last_error: Some("relay down".to_string()), account_email: None, sign_in_unreachable: false };
        let up = muxbus_answer(&status(MuxBusDeliveryState::Connected));
        assert_eq!((up["connected"].clone(), up["signed_in"].clone()), (json!(true), json!(true)));
        let down = muxbus_answer(&status(MuxBusDeliveryState::Reconnecting));
        assert_eq!((down["connected"].clone(), down["signed_in"].clone()), (json!(false), json!(true)));
        let out = muxbus_answer(&status(MuxBusDeliveryState::SignedOut));
        assert_eq!((out["state"].clone(), out["signed_in"].clone(), out["last_error"].clone()), (json!("signed_out"), json!(false), json!("relay down")));
    }

    #[test]
    fn an_agent_nobody_has_is_not_found_rather_than_an_error() {
        use crate::server::availability_view::Known;
        assert_eq!(agent_answer(Known::Unknown), json!({ "found": false }));
        assert_eq!(agent_answer(Known::Ambiguous), json!({ "found": false, "ambiguous": true }));
        assert_eq!(agent_answer(Known::Live(json!({ "state": "idle", "would": "deliver" }))), json!({ "state": "idle", "would": "deliver", "found": true }));
    }

    #[test]
    fn agents_say_how_many_are_up_and_which_wanted_running_ones_are_not() {
        use crate::backend::agent_runtime_state::{AgentDesiredState as D, AgentRuntime, AgentRuntimeState as S};
        let row = |name: &str, state: S, desired: D, detail: Option<&str>| AgentRuntime {
            agent_id: format!("uid-{name}"),
            state,
            detail: detail.map(str::to_string),
            resets_at: None,
            block_id: None,
            waiting_in: None,
            desired,
            name: name.to_string(),
            provider: "claude".to_string(),
            icon: String::new(),
            color: None,
            blocked: None,
            runtime: Default::default(),
            model_id: None,
            model_label: None,
            model_source: None,
            effort: None,
            pending_switch: None,
            definition_model_label: None,
        };
        let healthy = agents_answer(&[row("a", S::Idle, D::Running, None), row("b", S::Working, D::Running, None), row("c", S::WaitingOnUser, D::Running, None), row("off", S::Stopped, D::Stopped, None)]);
        assert_eq!((healthy["count"].clone(), healthy["wanted_running"].clone(), healthy["up"].clone(), healthy["all_up"].clone()), (json!(4), json!(3), json!(3), json!(true)), "an agent wanted stopped is no problem");
        assert_eq!(healthy["problems"], json!([]));
        let sick = agents_answer(&[row("a", S::Idle, D::Running, None), row("lost", S::Stopped, D::Running, None), row("out", S::SignedOut, D::Running, Some("needs_login")), row("slow", S::Starting, D::Running, None)]);
        assert_eq!((sick["wanted_running"].clone(), sick["up"].clone(), sick["all_up"].clone(), sick["problem_count"].clone()), (json!(4), json!(1), json!(false), json!(3)));
        assert_eq!(sick["problems"][1], json!({ "agent": "out", "state": "signed_out", "detail": "needs_login" }));
        assert_eq!(sick["problem_names"], json!("lost (stopped), out (signed_out), slow (starting)"));
        assert_eq!(agents_answer(&[])["all_up"], json!(true), "no agent wanted running is nothing wrong");
    }

    #[test]
    fn the_agents_query_has_no_filter() {
        assert_eq!(plan(&node(json!({ "query": "agents" })), &ExecutionScope::new()).unwrap(), Query::Agents);
    }
}
