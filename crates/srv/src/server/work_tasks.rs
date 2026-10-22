// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The agents' side of the Kanban (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md
//! §6.5, §6.8, §6.12), beside the queue's own routes in `work_queue.rs`:
//!
//!   POST /agentmux/work/:id/update   — a progress note, a checklist, links
//!   POST /agentmux/work/:id/ask      — a question for the operator (blocked)
//!   POST /agentmux/work/:id/submit   — hand it in (review, or done)
//!   POST /agentmux/work/:id/hold     — put a waiting task on hold, or pause
//!                                      one's own (any agent)
//!   POST /agentmux/work/:id/unhold   — take a held task off hold (any agent)
//!
//! And what srv adds to every task an agent touches: its card (number,
//! repository, who asked), its history, the next task waiting for the agent
//! when it finishes one, and leases that a live holder keeps without calling
//! `heartbeat`.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use agentmux_common::time::now_ms;

use crate::backend::kanban::prs::{link_if_from, pr_in_text};
use crate::backend::reactive::types::AgentRegistration;
use crate::backend::storage::work_board::{requested, BoardTask, CardUpdate, ChecklistItem, NewCard, WorkEvent};
use crate::backend::storage::work_queue::{work_state, ClaimFilter, WorkItem};

use super::caller::Caller;
use super::AppState;

/// The lease a live holder's claim is kept at (`work_queue::DEFAULT_LEASE_MS`).
const LEASE_MS: i64 = 120_000;
/// The longest note, question or result kept on a card.
const MAX_TEXT: usize = 4_000;
/// The most checklist steps kept.
const MAX_CHECKLIST: usize = 50;

/// A task's id from what an agent or the operator typed: its id, or its
/// number (`12`, `#12`).
pub(super) fn resolve_id(state: &AppState, id: &str) -> String {
    let trimmed = id.trim();
    let looks_numeric = trimmed.trim_start_matches('#').chars().all(|c| c.is_ascii_digit()) && !trimmed.is_empty();
    if looks_numeric {
        if let Ok(Some(t)) = state.identity_store.work_board_get(trimmed) {
            return t.item.id;
        }
    }
    trimmed.to_string()
}

/// The tasks a task starts after (§6.15), as given: ids or numbers (`12`,
/// `"#12"`), resolved to ids. The store checks the list when it writes it.
pub(super) fn resolve_after(state: &AppState, given: &[Value]) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = Vec::new();
    for v in given {
        let raw = match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return Err(format!("after: {v} is not a task id or number")),
        };
        let id = resolve_id(state, &raw);
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Text kept on a card or in the history: sanitized, at most [`MAX_TEXT`]
/// characters.
pub(super) fn clean(text: &str) -> String {
    let s = crate::backend::reactive::sanitize::sanitize_message(text.trim());
    match s.char_indices().nth(MAX_TEXT) {
        Some((cut, _)) => format!("{}…", &s[..cut]),
        None => s,
    }
}

/// One line of history; a failure to write it is logged, never the caller's.
pub(super) fn record(state: &AppState, work_id: &str, actor: &str, actor_kind: &str, kind: &str, from: &str, to: &str, body: &str) {
    let ev = WorkEvent {
        id: 0,
        work_id: work_id.to_string(),
        at: now_ms(),
        actor: actor.to_string(),
        actor_kind: actor_kind.to_string(),
        kind: kind.to_string(),
        from_state: from.to_string(),
        to_state: to.to_string(),
        body: clean(body),
    };
    if let Err(e) = state.identity_store.work_event_add(&ev) {
        tracing::warn!(target: "workqueue", error = %e, work_id, kind, "history line not written");
    }
}

/// The calling agent's registration, by its token only: who asked for a task
/// must not be something the caller can claim (§6.5).
fn caller_reg(state: &AppState, caller: Option<&Caller>) -> Option<AgentRegistration> {
    let uid = caller?.uid()?;
    state.reactive_handler.list_agents().into_iter().find(|r| r.uid.as_deref() == Some(uid))
}

/// Who asked for what the agent is doing now: the operator, when a message of
/// theirs started or joined the agent's current turn (with their words);
/// another agent, when its message started it; else other. `unknown` when the
/// call carried no token.
fn provenance(state: &AppState, reg: Option<&AgentRegistration>) -> (String, String, String) {
    let Some(reg) = reg else {
        return (String::new(), requested::UNKNOWN.into(), String::new());
    };
    let scope = format!("block:{}", reg.block_id);
    let turn = state
        .broker
        .read_event_history(crate::backend::mps::EVENT_AGENT_TURN, &scope, 1)
        .pop()
        .and_then(|e| e.data);
    let Some(turn) = turn else {
        return (String::new(), requested::OTHER.into(), String::new());
    };
    let kind_of = |t: &Value| t.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    let mut triggers: Vec<&Value> = turn.get("trigger").into_iter().collect();
    if let Some(abs) = turn.get("absorbed").and_then(Value::as_array) {
        triggers.extend(abs.iter());
    }
    if triggers.iter().any(|t| matches!(kind_of(t).as_str(), "user" | "broadcast")) {
        let text = state
            .mstore
            .get::<crate::backend::obj::Block>(&reg.block_id)
            .ok()
            .flatten()
            .map(|b| crate::backend::obj::meta_get_string(&b.meta, "term:last_prompt", ""))
            .unwrap_or_default();
        return ("operator".into(), requested::OPERATOR.into(), clean(&text));
    }
    let first = turn.get("trigger");
    if first.map(kind_of).as_deref() == Some("agent") {
        let from = first.and_then(|t| t.get("from")).and_then(Value::as_str).unwrap_or("").to_string();
        return (from, requested::AGENT.into(), String::new());
    }
    (String::new(), requested::OTHER.into(), String::new())
}

/// The repository the agent works in and the branch it is on, from its work
/// facts.
async fn repo_of(state: &AppState, reg: Option<&AgentRegistration>) -> (String, String) {
    let Some(reg) = reg.cloned() else { return Default::default() };
    let store = state.mstore.clone();
    let channel = crate::backend::work_facts::cross_channel::own_channel();
    let facts = tokio::task::spawn_blocking(move || crate::backend::work_facts::collect_local(&store, &[reg], &channel))
        .await
        .unwrap_or_default();
    facts
        .into_iter()
        .next()
        .map(|f| (f.repo_key().unwrap_or_default(), f.branch.unwrap_or_default()))
        .unwrap_or_default()
}

/// What an enqueue adds for the board: who asked, the repository.
pub(super) struct Filing {
    pub repo: Option<String>,
    pub review: Option<bool>,
}

/// The card of a task about to be enqueued: who asked, the repository, the
/// filer. Worked out before the row exists, so nothing awaits between the
/// row and its card (`file_card`).
pub(super) async fn prepare_card(state: &AppState, caller: Option<&Caller>, created_by: &str, f: Filing) -> NewCard {
    let reg = caller_reg(state, caller);
    let (requested_by, kind, request_text) = provenance(state, reg.as_ref());
    // Not the branch the filer is on: that is its current work's, not this
    // task's. The task's branch comes from the PR it opens (kanban::on_new_prs).
    let repo = match f.repo.filter(|r| !r.trim().is_empty()) {
        Some(r) => r.trim().to_string(),
        None => repo_of(state, reg.as_ref()).await.0,
    };
    let filer = reg.as_ref().map(|r| r.agent_id.clone()).unwrap_or_else(|| created_by.to_string());
    NewCard {
        repo,
        branch: String::new(),
        // What the operator asked for needs the operator's eyes; a handoff
        // between agents doesn't (D2).
        review_required: f.review.unwrap_or(kind == requested::OPERATOR),
        requested_by,
        requested_by_kind: kind,
        request_text,
        filed_by: filer,
    }
}

/// Give a just-enqueued task its card and first history line. Returns its
/// number. Synchronous: called right after the enqueue, with nothing awaited
/// between them; a claim or list that got in first made an empty card, which
/// this fills.
pub(super) fn file_card(state: &AppState, item: &WorkItem, card: &NewCard) -> Option<i64> {
    let now = now_ms();
    let filer = card.filed_by.clone();
    let num = match state.identity_store.work_card_create(&item.id, card, now) {
        Ok(n) => Some(n),
        Err(e) => {
            tracing::warn!(target: "workqueue", error = %e, "card not created");
            None
        }
    };
    let for_itself = !item.target_agent.is_empty() && item.target_agent == filer;
    let state_now = if item.state == work_state::HELD { work_state::HELD } else { work_state::OPEN };
    record(state, &item.id, &filer, "agent", if for_itself { "filed_for_later" } else { "created" }, "", state_now, "");
    num
}

/// The next task waiting for this agent, as the answer to finishing one
/// carries it (§6.5: it proceeds through its backlog).
pub(super) fn next_for(state: &AppState, agent_id: &str, uid: &str) -> Value {
    let waiting = state.identity_store.work_board_waiting_for(agent_id, uid, 50, now_ms()).unwrap_or_default();
    match waiting.first() {
        Some(t) => json!({
            "next": task_brief(t),
            "waiting": waiting.len(),
        }),
        None => json!({ "next": null, "waiting": 0 }),
    }
}

/// The few fields of a task an agent's answer names.
pub(super) fn task_brief(t: &BoardTask) -> Value {
    json!({
        "id": t.item.id,
        "num": t.card.num,
        "title": t.item.title,
        "requested_by": t.card.requested_by,
        "requested_by_kind": t.card.requested_by_kind,
        "created_at": t.item.created_at,
    })
}

/// Keep the leases of claims whose holder is alive, so a long tool call or a
/// model that never calls `heartbeat` doesn't lose its task (§6.8). A holder
/// is alive while it is registered and its process runs. Run before the reap.
pub(super) fn extend_live_leases(state: &AppState, now: i64) {
    let Ok(claimed) = state.identity_store.work_claimed_all() else { return };
    if claimed.is_empty() {
        return;
    }
    let regs = state.reactive_handler.list_agents();
    for item in claimed {
        let holder = regs.iter().find(|r| {
            (!item.claimed_by_uid.is_empty() && r.uid.as_deref() == Some(item.claimed_by_uid.as_str()))
                || (item.claimed_by_uid.is_empty() && r.agent_id == item.claimed_by)
        });
        let alive = holder.is_some_and(|r| {
            crate::backend::blockcontroller::get_block_controller_status(&r.block_id)
                .is_some_and(|s| s.shellprocstatus == crate::backend::blockcontroller::STATUS_RUNNING)
        });
        if alive {
            let _ = state.identity_store.work_extend_lease(&item.id, item.attempts, now, LEASE_MS);
        }
    }
}

/// The claims about to be reaped: record them as expired after the reap.
pub(super) fn expiring(state: &AppState, now: i64) -> Vec<WorkItem> {
    state
        .identity_store
        .work_claimed_all()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| i.claim_expires.is_some_and(|e| e <= now))
        .collect()
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateRequest {
    pub agent_id: String,
    pub attempt: i64,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub checklist: Option<Vec<ChecklistItem>>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
    /// `ask`: the question; `submit`: the result.
    #[serde(default)]
    pub text: String,
}

fn card_update(req: &UpdateRequest) -> CardUpdate {
    let one = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(clean);
    CardUpdate {
        note: one(&req.note),
        checklist: req.checklist.as_ref().map(|c| {
            c.iter()
                .take(MAX_CHECKLIST)
                .map(|i| ChecklistItem { text: clean(&i.text), status: i.status.trim().to_string() })
                .collect()
        }),
        branch: one(&req.branch),
        pr_url: one(&req.pr_url),
    }
}

fn holder_uid(caller: Option<&Caller>) -> &str {
    caller.and_then(Caller::uid).unwrap_or("")
}

fn conflict() -> (StatusCode, Json<Value>) {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "ok": false,
            "error": "not the current holder, or this claim has been superseded — re-claim before continuing"
        })),
    )
}

fn failed(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))
}

/// Whether the caller holds this claim now: the same rule as the queue's
/// holder transitions (UID when both are known, else the name), and the fence.
fn holds(item: &WorkItem, agent_id: &str, uid: &str, attempt: i64) -> bool {
    let who = if !item.claimed_by_uid.is_empty() && !uid.is_empty() {
        item.claimed_by_uid == uid
    } else {
        item.claimed_by == agent_id
    };
    who && item.attempts == attempt && matches!(item.state.as_str(), work_state::CLAIMED | work_state::BLOCKED)
}

/// `POST /agentmux/work/:id/update`: a progress note, a checklist, links. Also
/// a heartbeat.
pub(super) async fn handle_work_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<UpdateRequest>,
) -> (StatusCode, Json<Value>) {
    let caller = caller.as_deref();
    let id = resolve_id(&state, &id);
    let Ok(Some(item)) = state.identity_store.work_queue_get(&id) else { return conflict() };
    if !holds(&item, &req.agent_id, holder_uid(caller), req.attempt) {
        return conflict();
    }
    let now = now_ms();
    let _ = state.identity_store.work_card_ensure(&id, now);
    let u = card_update(&req);
    if let Err(e) = state.identity_store.work_card_update(&id, &u, now) {
        return failed(e);
    }
    let _ = state.identity_store.work_extend_lease(&id, item.attempts, now, LEASE_MS);
    let mut body = u.note.clone().unwrap_or_default();
    if let Some(c) = &u.checklist {
        let done = c.iter().filter(|i| i.status == "completed").count();
        body = format!("{body}{}checklist {done}/{}", if body.is_empty() { "" } else { " · " }, c.len());
    }
    record(&state, &id, &req.agent_id, "agent", "updated", &item.state, &item.state, &body);
    if u.pr_url.is_some() || u.branch.is_some() {
        let link = [u.pr_url.as_deref(), u.branch.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        record(&state, &id, &req.agent_id, "agent", "linked", &item.state, &item.state, &link);
    }
    super::work_queue::publish_changed(&state);
    (StatusCode::OK, Json(json!({ "ok": true })))
}

/// `POST /agentmux/work/:id/ask`: the holder needs the operator. The task
/// goes to Needs you; the holder keeps it, its lease paused.
pub(super) async fn handle_work_ask(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<UpdateRequest>,
) -> (StatusCode, Json<Value>) {
    let caller = caller.as_deref();
    let question = clean(&req.text);
    if question.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "the question is required" })));
    }
    let id = resolve_id(&state, &id);
    let now = now_ms();
    match state.identity_store.work_ask(&id, &req.agent_id, holder_uid(caller), req.attempt, now) {
        Ok(Some(_)) => {
            let _ = state.identity_store.work_card_ensure(&id, now);
            let _ = state.identity_store.work_card_set_question(&id, &question, now);
            record(&state, &id, &req.agent_id, "agent", "asked", work_state::CLAIMED, work_state::BLOCKED, &question);
            super::work_queue::publish_changed(&state);
            (StatusCode::OK, Json(json!({ "ok": true, "state": work_state::BLOCKED })))
        }
        Ok(None) => conflict(),
        Err(e) => failed(e),
    }
}

/// `POST /agentmux/work/:id/submit`: the holder hands it in. A task that
/// needs review waits in Review for the operator; one that doesn't is done.
/// The answer names the next task waiting for the agent.
pub(super) async fn handle_work_submit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<UpdateRequest>,
) -> (StatusCode, Json<Value>) {
    let caller = caller.as_deref();
    let result = clean(&req.text);
    if result.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "result is required: what was done, with links" })));
    }
    let id = resolve_id(&state, &id);
    finish(&state, caller, &id, &req.agent_id, req.attempt, &result, Some(card_update(&req))).await
}

#[derive(Debug, Deserialize)]
pub(super) struct HoldRequest {
    pub agent_id: String,
    /// `hold` on a task the caller is working on: the attempt from its claim.
    #[serde(default)]
    pub attempt: Option<i64>,
    #[serde(default)]
    pub reason: String,
}

/// A task as an agent's answer names it: `#12`, else its id.
fn name_of(state: &AppState, id: &str) -> String {
    match state.identity_store.work_card_ensure(id, now_ms()) {
        Ok(n) => format!("#{n}"),
        Err(_) => id.to_string(),
    }
}

/// Who a waiting task is for, as an answer says it.
fn waiting_for(item: &WorkItem) -> String {
    if !item.target_agent.is_empty() {
        item.target_agent.clone()
    } else if !item.target_group.is_empty() {
        format!("group {}", item.target_group)
    } else {
        "anyone".into()
    }
}

/// A hold's reason shows on the shared Kanban: one that looks like a
/// credential or a destructive command is refused, before anything moves.
fn sensitive_reason(reason: &str) -> Option<(StatusCode, Json<Value>)> {
    crate::backend::reactive::sanitize::is_sensitive_message(reason).then(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "the reason looks like it holds a credential or a destructive command; leave that out of it" })),
        )
    })
}

fn refused(text: String) -> (StatusCode, Json<Value>) {
    (StatusCode::CONFLICT, Json(json!({ "ok": false, "error": text })))
}

fn moved(state: &AppState, id: &str, to: &str, changed: bool) -> (StatusCode, Json<Value>) {
    let item = state.identity_store.work_queue_get(id).ok().flatten();
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "state": to,
            "changed": changed,
            "name": name_of(state, id),
            "for": item.as_ref().map(waiting_for).unwrap_or_default(),
        })),
    )
}

/// `POST /agentmux/work/:id/hold`: any agent puts a waiting task on hold, so
/// no one takes it until it is taken off hold; the holder of a claimed task
/// pauses it, with its `attempt`
/// (SPEC_KANBAN_AGENTS_HOLD_AND_UNHOLD_2026_10_22.md §3.1, §3.2).
pub(super) async fn handle_work_hold(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<HoldRequest>,
) -> (StatusCode, Json<Value>) {
    let caller = caller.as_deref();
    if req.agent_id.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "agent_id is required" })));
    }
    let id = resolve_id(&state, &id);
    let Ok(Some(item)) = state.identity_store.work_queue_get(&id) else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("no task {id}") })));
    };
    let reason = clean(&req.reason);
    if let Some(r) = sensitive_reason(&reason) {
        return r;
    }
    let now = now_ms();
    let name = name_of(&state, &id);
    match item.state.as_str() {
        work_state::HELD => moved(&state, &id, work_state::HELD, false),
        work_state::OPEN => match state.identity_store.work_hold(&id, now) {
            Ok(Some(m)) => {
                record(&state, &id, &req.agent_id, "agent", "held", &m.from, &m.to, &reason);
                super::work_queue::publish_changed(&state);
                moved(&state, &id, work_state::HELD, true)
            }
            Ok(None) => refused(format!("{name} moved meanwhile; look at it again with Tasks list")),
            Err(e) => failed(e),
        },
        work_state::CLAIMED => {
            let uid = holder_uid(caller);
            let Some(attempt) = req.attempt.filter(|&a| holds(&item, &req.agent_id, uid, a)) else {
                let who = if item.claimed_by.is_empty() { "another agent".to_string() } else { item.claimed_by.clone() };
                return refused(format!(
                    "{name} is being worked on by {who}: only its holder can put it on hold (with its attempt); message them"
                ));
            };
            match state.identity_store.work_hold_own(&id, &req.agent_id, uid, attempt, now) {
                Ok(Some(_)) => {
                    record(&state, &id, &req.agent_id, "agent", "held", work_state::CLAIMED, work_state::HELD, &reason);
                    super::work_queue::publish_changed(&state);
                    moved(&state, &id, work_state::HELD, true)
                }
                Ok(None) => conflict(),
                Err(e) => failed(e),
            }
        }
        work_state::BLOCKED => refused(format!(
            "{name} waits on the operator's answer to its question; that is already its pause"
        )),
        other => refused(format!(
            "{name} is {other}: only a waiting task, or one you are working on, can be put on hold"
        )),
    }
}

/// `POST /agentmux/work/:id/unhold`: any agent takes a held task off hold; it
/// waits again for whoever it is for (§3.1).
pub(super) async fn handle_work_unhold(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<HoldRequest>,
) -> (StatusCode, Json<Value>) {
    if req.agent_id.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "agent_id is required" })));
    }
    let id = resolve_id(&state, &id);
    let Ok(Some(item)) = state.identity_store.work_queue_get(&id) else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("no task {id}") })));
    };
    let reason = clean(&req.reason);
    if let Some(r) = sensitive_reason(&reason) {
        return r;
    }
    let name = name_of(&state, &id);
    match item.state.as_str() {
        work_state::OPEN => moved(&state, &id, work_state::OPEN, false),
        work_state::HELD => match state.identity_store.work_unhold(&id, now_ms()) {
            Ok(Some(m)) => {
                record(&state, &id, &req.agent_id, "agent", "unheld", &m.from, &m.to, &reason);
                super::work_queue::publish_changed(&state);
                moved(&state, &id, work_state::OPEN, true)
            }
            Ok(None) => refused(format!("{name} moved meanwhile; look at it again with Tasks list")),
            Err(e) => failed(e),
        },
        other => refused(format!("{name} is {other}, not on hold")),
    }
}

/// Why a named claim (`claim` with `id`) took nothing: the first rule the task
/// fails, in words the agent can act on.
pub(super) fn why_not_claimed(state: &AppState, id: &str, filter: &ClaimFilter, now: i64) -> String {
    let (agent_id, agent_uid, groups) = (filter.agent_id.as_str(), filter.agent_uid.as_str(), &filter.groups);
    let Ok(Some(item)) = state.identity_store.work_queue_get(id) else {
        return format!("there is no task {id}");
    };
    let name = name_of(state, id);
    if item.state != work_state::OPEN && item.state != work_state::HELD {
        let who = if item.claimed_by.is_empty() { String::new() } else { format!(" (by {})", item.claimed_by) };
        return format!("{name} is {}{who}, not waiting", item.state);
    }
    let mine = if !item.target_agent_uid.is_empty() {
        item.target_agent_uid == agent_uid
    } else {
        item.target_agent.is_empty() || item.target_agent == agent_id
    };
    if !mine {
        return format!("{name} is for {}, not you", waiting_for(&item));
    }
    if !item.target_group.is_empty() && !groups.contains(&item.target_group) {
        return format!("{name} is for group {}, which you aren't in", item.target_group);
    }
    if let Some(kind) = filter.kind.as_deref().filter(|k| !k.is_empty() && *k != item.kind) {
        return format!("{name} is not of kind {kind}");
    }
    if let Some(after) = state.identity_store.work_deps_of(id).unwrap_or_default().into_iter().find(|a| !a.finished()) {
        return format!("{name} starts after #{} \"{}\", which is {}", after.num, after.title, after.state);
    }
    if item.attempts >= item.max_attempts {
        return format!("{name} has used all {} of its attempts", item.max_attempts);
    }
    if item.not_before.is_some_and(|t| t > now) {
        return format!("{name} can't be taken yet: its start time hasn't come");
    }
    format!("{name} moved meanwhile; look at it again with Tasks list")
}

/// Finish a claimed task, for `complete` and `submit`: to Review when it needs
/// review (the agent can't accept its own work), else done. Answers with the
/// resulting state and the agent's next waiting task.
pub(super) async fn finish(
    state: &AppState,
    caller: Option<&Caller>,
    id: &str,
    agent_id: &str,
    attempt: i64,
    result: &str,
    update: Option<CardUpdate>,
) -> (StatusCode, Json<Value>) {
    let now = now_ms();
    let uid = holder_uid(caller);
    let review = state
        .identity_store
        .work_board_get(id)
        .ok()
        .flatten()
        .is_some_and(|t| t.card.review_required);
    let before = state.identity_store.work_queue_get(id).ok().flatten().map(|i| i.state).unwrap_or_default();
    let matched = if review {
        state.identity_store.work_submit(id, agent_id, uid, attempt, result, now)
    } else {
        state.identity_store.work_queue_complete(id, agent_id, uid, attempt, result, now)
    };
    match matched {
        Ok(Some(_)) => {
            if let Some(u) = update {
                let _ = state.identity_store.work_card_ensure(id, now);
                let _ = state.identity_store.work_card_update(id, &u, now);
            }
            // A result naming one PR links it, so the card shows it and its
            // merge can close the task, once GitHub says it is from the
            // branch the agent is on (kanban::prs).
            if let Ok(Some(t)) = state.identity_store.work_board_get(id) {
                if let Some(pr) = t.card.pr_url.is_empty().then(|| pr_in_text(result, &t.card.repo)).flatten() {
                    let (repo, branch) = repo_of(state, caller_reg(state, caller).as_ref()).await;
                    link_if_from(id.to_string(), pr, repo, branch);
                }
            }
            let _ = state.identity_store.work_card_set_question(id, "", now);
            let to = if review { work_state::REVIEW } else { work_state::DONE };
            record(state, id, agent_id, "agent", if review { "submitted" } else { "completed" }, &before, to, result);
            super::work_queue::publish_changed(state);
            let holder_uid = state.identity_store.work_queue_get(id).ok().flatten().map(|i| i.claimed_by_uid).unwrap_or_default();
            let next_uid = if uid.is_empty() { holder_uid } else { uid.to_string() };
            let mut body = json!({ "ok": true, "state": to });
            // An agent the operator switched off going on (`kanban:continue`)
            // is told of no next task.
            let goes_on = caller_reg(state, caller).is_none_or(|r| crate::backend::kanban::continues(&state.mstore, &r.block_id));
            if let (true, Some(obj), Value::Object(next)) = (goes_on, body.as_object_mut(), next_for(state, agent_id, &next_uid)) {
                obj.extend(next);
            }
            (StatusCode::OK, Json(body))
        }
        Ok(None) => conflict(),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
#[path = "work_tasks_tests.rs"]
mod tests;
