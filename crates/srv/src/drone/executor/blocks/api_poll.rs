// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The question a `poll` trigger asks when its `query` is `api` (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md
//! §3.2): one GET of a site's list, with an optional Vault credential, answered as the items in it.
//!
//! The call is the API block's own ([`super::api::run`]), not a second HTTP client, so a poll has the same site binding,
//! the same refusal of a `Host` header, the same header handling, the same size cap and timeout, and a credentialed call
//! follows no redirect (a 3xx is an error here, said so). What differs is one thing: a poll has nobody at the desk, so it
//! never asks the person. With a credential it uses only an "Always" the person already gave this drone for this site
//! (`ExecutionScope::never_ask`); without one it fails at once, with no card.
//!
//! The filter keys: `url` (https, written out), `auth` (as the API block's), `headers`, `items` (the dotted path to the
//! list in the answer, empty when the answer is the list), `id` (the dotted path of an item's id, `id` when left out),
//! `limit` (at most that many items kept, 100 when left out). What comes back is outside text: each item is the answer's
//! own, and goes to the run as `{{trigger.item}}` like the API block's output, with the same handling.

use serde_json::{json, Map, Value};

use crate::drone::data_flow::ExecutionScope;
use crate::drone::types::FlowNode;

use crate::backend::jobs::poll::{API_DEFAULT_ITEMS as DEFAULT_ITEMS, API_MAX_ITEMS as MAX_ITEMS};

/// The value at the dotted `path` of `v`; the value itself for an empty path. Array positions are numbers (`a.0.b`).
fn at<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for key in path.split('.').map(str::trim).filter(|k| !k.is_empty()) {
        cur = match cur {
            Value::Object(m) => m.get(key)?,
            Value::Array(a) => a.get(key.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// An item's id as text: a non-empty string or a number.
fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The items in a successful answer's body, each with its id: `{ "items": [ { "id": "…", "item": <the item> } ] }`.
/// An item with no id is not one. Pure, so it is testable without a server.
pub(crate) fn items_in(body: &Value, items_path: &str, id_path: &str, limit: usize) -> Result<Value, String> {
    let list = at(body, items_path).and_then(Value::as_array).ok_or_else(|| {
        if items_path.trim().is_empty() {
            "the answer is not a list: give `items`, the path to the list in it".to_string()
        } else {
            format!("the answer has no list at `{items_path}`")
        }
    })?;
    let id_path = if id_path.trim().is_empty() { "id" } else { id_path };
    let limit = limit.clamp(1, MAX_ITEMS);
    let items: Vec<Value> = list
        .iter()
        .filter_map(|item| at(item, id_path).and_then(id_text).map(|id| json!({ "id": id, "item": item })))
        .take(limit)
        .collect();
    Ok(json!({ "items": items }))
}

/// Asks the site `filter["url"]` for its list. `filter["drone_id"]` is the drone whose job polls, set by the poller: it is
/// who a Vault credential is granted to.
pub(crate) async fn ask(filter: Map<String, Value>) -> Result<Value, String> {
    let text = |k: &str| filter.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let url = text("url");
    if url.is_empty() {
        return Err("a poll of api has a url".to_string());
    }
    let mut data = Map::new();
    data.insert("method".into(), json!("GET"));
    data.insert("url".into(), json!(url));
    if let Some(h) = filter.get("headers") {
        data.insert("headers".into(), h.clone());
    }
    let has_auth = filter.get("auth").is_some_and(|a| !a.is_null());
    if has_auth {
        data.insert("auth".into(), filter["auth"].clone());
    }
    let drone_id = text("drone_id");
    if has_auth && drone_id.is_empty() {
        return Err("a poll with a Vault credential belongs to a drone job: the credential is the drone's".to_string());
    }
    let mut scope = ExecutionScope::new();
    scope.drone_id = drone_id;
    scope.never_ask = true;
    let node = FlowNode { id: "poll".to_string(), position: Default::default(), data: Value::Object(data), node_type: None };
    let out = super::api::run(&node, &scope).await?;
    let limit = filter.get("limit").and_then(Value::as_u64).map_or(DEFAULT_ITEMS, |n| n as usize);
    items_of_output(&out, &text("items"), &text("id"), limit)
}

/// The items in the API block's output: an error for any answer but a 2xx with a list in it.
pub(crate) fn items_of_output(out: &Value, items_path: &str, id_path: &str, limit: usize) -> Result<Value, String> {
    let status = out.get("status").and_then(Value::as_u64).unwrap_or(0);
    if !(200..300).contains(&status) {
        let note = if (300..400).contains(&status) {
            " (a call with a credential follows no redirect, so a redirect is an error: poll the address it moves to)"
        } else {
            ""
        };
        return Err(format!("the site answered {status}{note}"));
    }
    let body = match out.get("body") {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).map_err(|_| "the answer is not JSON".to_string())?,
        Some(other) => other.clone(),
        None => Value::Null,
    };
    items_in(&body, items_path, id_path, limit)
}

#[cfg(test)]
#[path = "api_poll_tests.rs"]
mod tests;
