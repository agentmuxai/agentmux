// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! App query block: reads AgentMux's own state, read-only (the Drone real-world jobs spec §4.2).
//! Deterministic and free: no agent turn, no network.
//!
//! Block config (`node.data`):
//!   * `query`   required, one of:
//!       - `muxbus`  whether cloud messages are reaching this machine.
//!       - `agent`   one agent's availability. Needs `agent`, its name, written in the graph.
//!       - `tasks`   Kanban tasks, filtered by `holder`, `state` and `pr` (each optional).
//!       - `lan_peers`  the other machines this one sees on the LAN, and whether each answered lately.
//!       - `jobs`    recent job runs: one job's with `job`, else the newest run of every job.
//!   * `agent`, `job`   the agent's or the job's name (a job's id works too). Never a `{{reference}}`:
//!               a value from outside must not pick whose state the graph reads (§6.2).
//!   * `holder`, `state`, `pr`   task filters. They only find tasks, so each may be built from a
//!               `{{reference}}` (a PR url from the trigger). `pr` matches a task's PR link exactly.
//!   * `limit`   tasks and jobs, how many to return (tasks: default 50, at most 200; jobs: default 20, at most 100).
//!
//! Output, by query:
//!   * `muxbus`: `{ "state", "connected", "signed_in", "since_ms", "last_ok_ms"?, "last_error"? }`.
//!   * `agent`:  `{ "found": bool, "ambiguous"?: true, "state"?, "would"?, "reason"?, ... }`, the
//!               availability the Agents tool shows. `found` is false for a name no agent here has.
//!   * `tasks`:  `{ "count": N, "tasks": [{ "num", "title", "state", "holder", "pr", "branch" }] }`.
//!   * `lan_peers`: `{ "count": N, "answered": M, "peers": [{ "host", "channel", "version", "os", "agents",
//!               "last_seen_ms", "last_answered_ms"?, "answered" }] }`. `answered` is true when the peer
//!               answered this machine's own poll (asked every 30 s) in the last two minutes: it is up and
//!               reachable from here. It is not a signature check.
//!   * `jobs`:   `{ "found": bool, "count": N, "runs": [{ "id", "job_id", "job", "state", "outcome", "trigger",
//!               "summary", "started_ms", "ended_ms"?, "cost_usd"? }] }`; `found` is false for a `job` that
//!               matches none.

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

/// What the block asks.
#[derive(Debug, PartialEq)]
pub(crate) enum Query {
    MuxBus,
    Agent(String),
    Tasks(TaskFilter),
    LanPeers,
    Jobs(JobsFilter),
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

/// What the node asks, with the filters' references resolved. Pure, so it is testable without a server.
pub(crate) fn plan(node: &FlowNode, scope: &ExecutionScope) -> Result<Query, String> {
    let text = |key: &str| node.data.get(key).and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
    match text("query") {
        "muxbus" => Ok(Query::MuxBus),
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
        "" => Err("app query block missing `query`".to_string()),
        other => Err(format!("app query: unknown query {other:?}; this AgentMux has muxbus, agent, tasks, lan_peers and jobs")),
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

/// `jobs`: the named job's newest runs, or the newest run of every job, newest first, cut to the limit.
pub(crate) fn jobs_answer(jobs: &crate::backend::storage::store::Store, ledger: &crate::backend::storage::store::Store, f: &JobsFilter) -> Result<Value, String> {
    let all = jobs.cron_list().map_err(|e| format!("app query: couldn't read the jobs: {e}"))?;
    let read = |e: crate::backend::storage::StoreError| format!("app query: couldn't read the runs: {e}");
    if f.job.is_empty() {
        let mut runs = Vec::new();
        for job in &all {
            runs.extend(ledger.job_run_last(&job.id).map_err(read)?);
        }
        runs.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
        let rows: Vec<Value> = runs.iter().take(f.limit).map(run_row).collect();
        return Ok(json!({ "found": true, "count": rows.len(), "runs": rows }));
    }
    let mut named = all.iter().filter(|j| j.id == f.job || j.name == f.job);
    let (Some(job), None) = (named.next(), named.next()) else {
        let more_than_one = all.iter().filter(|j| j.id == f.job || j.name == f.job).count() > 1;
        return Ok(if more_than_one { json!({ "found": false, "ambiguous": true, "count": 0, "runs": [] }) } else { json!({ "found": false, "count": 0, "runs": [] }) });
    };
    let runs = ledger.job_runs_for_job(&job.id, f.limit as i64, None).map_err(read)?;
    let rows: Vec<Value> = runs.iter().map(run_row).collect();
    Ok(json!({ "found": true, "count": rows.len(), "runs": rows }))
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
        assert!(err.contains("muxbus, agent, tasks, lan_peers and jobs"), "{err}");
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
}
