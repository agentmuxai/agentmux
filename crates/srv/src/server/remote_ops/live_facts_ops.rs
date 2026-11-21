// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agents.status` (live facts spec §6.2): this machine's agents with the facts a peer asked for, the answer the
//! discovery route gives a local agent (`live_facts::handler::answer`), as a signed read. It carries state, model and
//! counts only: no message text, prompt, transcript, credential or account email, and no pane or folder of this
//! machine.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::server::live_facts::handler::{answer, is_status_view};

/// The most agents one answer carries when the asker names no page: a long fleet is read in pages.
const DEFAULT_LIMIT: u64 = 200;

fn bad(message: impl Into<String>) -> RemoteRefusal {
    RemoteRefusal::new("bad_params", format!("agents.status: {}", message.into()))
}

/// The query the discovery answer takes, from the op's params: text or numbers by name. The asker's `host` is dropped:
/// the machine that answers is this one.
fn query_of(params: &Value) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    let map = match params {
        Value::Null => return Ok(out),
        Value::Object(map) => map,
        _ => return Err("the params are an object of the query's names".to_string()),
    };
    for (key, value) in map.iter().filter(|(k, _)| k.as_str() != "host") {
        let text = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Null => continue,
            _ => return Err(format!("{key} is text or a number")),
        };
        out.insert(key.clone(), text);
    }
    Ok(out)
}

/// What a peer does not learn of a row: where its pane or its folder is on this machine.
fn without_local_places(row: &mut Value) {
    let Some(fields) = row.as_object_mut() else { return };
    fields.remove("working_directory");
    fields.remove("block_id");
    if let Some(runtime) = fields.get_mut("runtime").and_then(Value::as_object_mut) {
        runtime.remove("block_id");
        runtime.remove("waiting_in");
    }
}

/// `agents.status`: `{host, version, as_of_ms, view, answer}`; the answer is the compact status for `view: status`,
/// else `{agents, page}`.
pub(super) async fn status(state: &AppState, _caller: &RemoteCaller, params: Value) -> RemoteResult {
    let mut query = query_of(&params).map_err(bad)?;
    query.entry("limit".to_string()).or_insert_with(|| DEFAULT_LIMIT.to_string());
    let compact = is_status_view(&query).map_err(bad)?;
    let body = answer(state, &query).await.map_err(bad)?;
    let answer = if compact {
        body
    } else {
        let mut agents = body.pointer("/host/agents").cloned().unwrap_or_else(|| json!([]));
        agents.as_array_mut().into_iter().flatten().for_each(without_local_places);
        json!({ "agents": agents, "page": body.pointer("/host/page").cloned().unwrap_or(Value::Null) })
    };
    Ok(json!({
        "host": state.hostname,
        "version": state.version,
        "as_of_ms": agentmux_common::time::now_ms_u64(),
        "view": if compact { "status" } else { "list" },
        "answer": answer,
    }))
}

#[cfg(test)]
#[path = "live_facts_ops_tests.rs"]
mod tests;
