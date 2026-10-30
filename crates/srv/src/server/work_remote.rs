// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Kanban's remote operations: `tasks.enqueue` and `tasks.list` on this machine's board, for an
//! agent of another machine (SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md §4.3, phase P3).
//!
//! The transport (`remote_ops`) has already proven the requesting machine and the request; these
//! functions take the verified requester's name (`slug@machine`) and the typed params, and do what the
//! same call does locally, through the same code (`work_queue::enqueue_core`, `list_items`). They never
//! see the envelope and write no audit row; the transport does both.
//!
//! What differs from a local call, on purpose:
//! - The task is asked for by that agent (`slug@machine`, kind `agent`), never by the operator, and is
//!   not held for the operator's Accept unless the request says `review: true`.
//! - The target is named by its slug or name and resolved here, on the machine that owns it; a name that
//!   matches nobody, or several agents, is refused with the candidates, never guessed.
//! - The request's `reason` isn't a parameter: it never reaches an agent (the transport keeps it for the audit).
//! - There is no `target_group`: a group is a local idea.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::work_queue::{enqueue_core, list_items, EnqueueRequest, Filer};
use super::AppState;

/// Why an operation didn't run: a short snake_case `code` (the transport's `refused_reason`) and words.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub code: &'static str,
    pub message: String,
}

fn fail(code: &'static str, message: impl Into<String>) -> Failure {
    Failure { code, message: message.into() }
}

/// The params as `T`, naming the field that is wrong or unknown.
fn params_of<T: DeserializeOwned>(params: Value) -> Result<T, Failure> {
    serde_json::from_value(params).map_err(|e| fail("bad_params", e.to_string()))
}

/// What `tasks.enqueue` takes: the local tool's fields, less `target_group`, `created_by` and the carried uid.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnqueueParams {
    title: String,
    payload: String,
    #[serde(default)]
    kind: String,
    /// The agent it is for, by slug, name or uid on THIS machine. Empty: for anyone.
    #[serde(default)]
    target_agent: String,
    #[serde(default)]
    priority: i64,
    #[serde(default)]
    max_attempts: Option<i64>,
    #[serde(default)]
    not_before: Option<i64>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    hold: bool,
    #[serde(default)]
    review: Option<bool>,
    /// Tasks on this machine's board it starts after, by number or id.
    #[serde(default)]
    after: Vec<Value>,
    #[serde(default)]
    first: Option<String>,
    #[serde(default)]
    ui_change: Option<bool>,
}

/// The uid of the agent here that `typed` names, or why none can be named.
async fn target_uid(state: &AppState, typed: &str) -> Result<String, Failure> {
    use crate::backend::name_resolution::NameResolution;
    // The typed name is interpreted at the boundary layer, never here.
    let resolved = super::name_resolution::resolve_typed_name(state, typed).await.map_err(|e| fail("failed", e))?;
    match resolved {
        NameResolution::One { uid, .. } => Ok(uid),
        NameResolution::Ambiguous { candidates } => {
            let names: Vec<String> = candidates.iter().map(|c| if c.slug.is_empty() { c.name.clone() } else { c.slug.clone() }).collect();
            Err(fail("ambiguous_target", format!("\"{typed}\" names more than one agent here: {}. Name one by its slug", names.join(", "))))
        }
        // An agent with no stored identity can't be given a task that only it may claim.
        NameResolution::Unidentified { .. } | NameResolution::None => Err(fail("no_such_agent", format!("no agent named \"{typed}\" on this machine"))),
    }
}

/// `tasks.enqueue`: file a task (or, with `first`, a spec and its build) on this machine's board for one of
/// its agents. The answer is the local call's: `{id, num}`, and `build_id`/`build_num` for a pair, with a
/// note when the target can't work now.
pub(crate) async fn tasks_enqueue(state: &AppState, actor: &str, params: Value) -> Result<Value, Failure> {
    let p: EnqueueParams = params_of(params)?;
    let target = p.target_agent.trim().to_string();
    let target_agent_uid = if target.is_empty() { String::new() } else { target_uid(state, &target).await? };
    let req = EnqueueRequest {
        title: p.title,
        payload: p.payload,
        kind: p.kind,
        target_agent: target,
        target_agent_uid,
        target_group: String::new(),
        priority: p.priority,
        created_by: actor.to_string(),
        max_attempts: p.max_attempts,
        not_before: p.not_before,
        repo: p.repo,
        hold: p.hold,
        review: p.review,
        after: p.after,
        first: p.first,
        ui_change: p.ui_change,
    };
    let (status, axum::response::Json(body)) = enqueue_core(state, req, Filer::Remote(actor)).await;
    if status.is_success() {
        return Ok(body);
    }
    let why = body.get("error").and_then(Value::as_str).unwrap_or("the task was not filed").to_string();
    Err(if status == axum::http::StatusCode::BAD_REQUEST { fail("bad_params", why) } else { fail("failed", why) })
}

/// What `tasks.list` takes.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListParams {
    /// open, held, claimed, blocked, review, done, failed or cancelled; all when absent.
    #[serde(default)]
    state: String,
    #[serde(default)]
    limit: Option<usize>,
}

/// `tasks.list`: this machine's board, read-only: `{items: [...]}`, each with its number.
pub(crate) fn tasks_list(state: &AppState, params: Value) -> Result<Value, Failure> {
    // No params at all is a plain "list".
    let p: ListParams = if params.is_null() { ListParams::default() } else { params_of(params)? };
    let items = list_items(state, &p.state, p.limit).map_err(|e| fail("failed", e))?;
    Ok(json!({ "items": items }))
}

#[cfg(test)]
#[path = "work_remote_tests.rs"]
mod tests;
