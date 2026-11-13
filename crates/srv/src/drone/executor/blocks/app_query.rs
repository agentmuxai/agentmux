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
//!   * `agent`   the agent's name. Never a `{{reference}}`: a value from outside must not pick
//!               whose state the graph reads (§6.2).
//!   * `holder`, `state`, `pr`   task filters. They only find tasks, so each may be built from a
//!               `{{reference}}` (a PR url from the trigger). `pr` matches a task's PR link exactly.
//!   * `limit`   tasks only, how many to return (default 50, at most 200).
//!
//! Output, by query:
//!   * `muxbus`: `{ "state", "connected", "signed_in", "since_ms", "last_ok_ms"?, "last_error"? }`.
//!   * `agent`:  `{ "found": bool, "ambiguous"?: true, "state"?, "would"?, "reason"?, ... }`, the
//!               availability the Agents tool shows. `found` is false for a name no agent here has.
//!   * `tasks`:  `{ "count": N, "tasks": [{ "num", "title", "state", "holder", "pr", "branch" }] }`.

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

/// What the block asks.
#[derive(Debug, PartialEq)]
pub(crate) enum Query {
    MuxBus,
    Agent(String),
    Tasks(TaskFilter),
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
        "" => Err("app query block missing `query`".to_string()),
        other => Err(format!("app query: unknown query {other:?}; this AgentMux has muxbus, agent and tasks")),
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
        let err = plan(&node(json!({ "query": "jobs" })), &s).unwrap_err();
        assert!(err.contains("muxbus, agent and tasks"), "{err}");
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
