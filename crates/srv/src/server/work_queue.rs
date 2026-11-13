// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Muxqueue HTTP surface — slice 2 of
//! `docs/reports/REPORT_UNIVERSAL_AGENT_WORK_QUEUE_2026_09_01.md`.
//!
//!   POST   /agentmux/work                 — enqueue
//!   POST   /agentmux/work/claim           — claim the next eligible item
//!   POST   /agentmux/work/:id/heartbeat   — extend a lease
//!   POST   /agentmux/work/:id/complete    — finish
//!   POST   /agentmux/work/:id/release     — give back
//!   GET    /agentmux/work                 — list
//!
//! The Kanban's additions (cards, history, update/ask/submit, the next task)
//! are in `work_tasks.rs` (SPEC_WARDEN_KANBAN_2026_10_20.md). An `:id` may be
//! a task's number (`12`) as well as its id.
//!
//! Auth-gated by the same middleware as the cron and reactive routes.
//!
//! **Store choice:** every handler uses `state.work_store`. In an installed or
//! `task package` build that is the always-global identity store, not
//! `state.shared_store` (cron's place until it moves, `SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`
//! step 1b), and deliberately so: a per-channel queue would only ever mean
//! "any agent in this channel can pick it up". A `task dev` build is the one
//! exception, with a `work-store.db` of its own (`crate::registry::work_store`),
//! so a test there never reads or changes the operator's real queue.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::backend::storage::work_queue::{ClaimFilter, HolderMatch, WorkItem};

use super::AppState;

/// Default lease granted by a claim, and the window a heartbeat extends by.
/// Deliberately short relative to a long agent turn: a claimant is expected to
/// heartbeat, and a crashed one should return to the pool in about a minute
/// rather than blocking the item for an hour.
const DEFAULT_LEASE_MS: i64 = 120_000;

use agentmux_common::time::now_ms;

pub(crate) fn publish_changed(state: &AppState) {
    state.broker.publish_event::<crate::backend::mps_events::WorkQueueChanged>(&(), vec![], 0);
}

fn err(code: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg.into() })))
}

#[derive(Debug, Deserialize)]
pub(super) struct EnqueueRequest {
    pub title: String,
    pub payload: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub target_agent: String,
    /// Identity M3: the target's UID, resolved at the MCP boundary (spec
    /// §5) and CARRIED here. When present it is stored as-is; when absent
    /// the server resolves `target_agent` itself as a counted fallback.
    #[serde(default)]
    pub target_agent_uid: String,
    #[serde(default)]
    pub target_group: String,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub max_attempts: Option<i64>,
    /// ms epoch. Not claimable before this.
    #[serde(default)]
    pub not_before: Option<i64>,
    /// Kanban: the repository it belongs to; default the enqueuer's own.
    #[serde(default)]
    pub repo: Option<String>,
    /// Kanban: on hold until the operator releases it.
    #[serde(default)]
    pub hold: bool,
    /// Kanban: whether a person must accept it; default from who asked.
    #[serde(default)]
    pub review: Option<bool>,
    /// Kanban: the tasks it starts after (ids or numbers).
    #[serde(default)]
    pub after: Vec<Value>,
    /// Kanban: `"spec"` or `"research"` files that task and a build that
    /// starts after it, for the same agent
    /// (SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §5.1).
    #[serde(default)]
    pub first: Option<String>,
    /// Kanban: the task changes the UI, so it needs media to be handed in for review
    /// (SPEC_KANBAN_TASK_MEDIA_2026_10_25.md §5).
    #[serde(default)]
    pub ui_change: Option<bool>,
    /// Which board to file it on, by name or slug; the default board when absent.
    #[serde(default)]
    pub board: Option<String>,
}

/// Who files a task: a caller on this machine (its token says who it is and what it was asked), or an
/// agent of another machine, named `slug@machine`, whose request the transport has verified.
pub(super) enum Filer<'a> {
    Local(Option<&'a super::caller::Caller>),
    Remote(&'a str),
}

pub(super) async fn handle_work_enqueue(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<EnqueueRequest>,
) -> (StatusCode, Json<Value>) {
    super::actor::check_actor(
        &state,
        caller.as_deref(),
        super::actor::ActorSite::WorkEnqueue,
        Some(&req.created_by),
    );
    enqueue_core(&state, req, Filer::Local(caller.as_deref())).await
}

/// File a task (or a spec and its build). The one path for a local call and a remote operation, so a
/// task filed from another machine is numbered, carded and noted exactly like a local one.
pub(super) async fn enqueue_core(state: &AppState, req: EnqueueRequest, filer: Filer<'_>) -> (StatusCode, Json<Value>) {
    let state = state.clone();
    let (caller, remote) = match filer {
        Filer::Local(c) => (c, None),
        Filer::Remote(actor) => (None, Some(actor.to_string())),
    };
    if req.title.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "title is required");
    }
    if req.payload.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "payload is required");
    }
    // An item targeted at both a specific agent AND a group is ambiguous —
    // reject rather than silently letting one win, which would look like the
    // queue quietly ignored half the caller's intent.
    if !req.target_agent.is_empty() && !req.target_group.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            "specify target_agent or target_group, not both",
        );
    }

    // Identity M3: the target's UID is stored only if the caller CARRIED
    // one — resolved at the boundary where the name was typed (the MCP, via
    // `POST /agentmux/agents/resolve`, spec §5). This handler never turns a
    // name into an identity itself (§2 rule 1): since M3 the UID decides who
    // may claim the item, and a server-side guess — the host-wide slug
    // registry, a template id — would bind it to whichever agent the guess
    // found, in any channel (review on #3563). A named target with no
    // carried UID keeps the name path until M5, counted (§9.2).
    let target_agent_uid = req.target_agent_uid.trim().to_string();
    if target_agent_uid.is_empty() && !req.target_agent.trim().is_empty() {
        crate::backend::agent_resolve::record_uid_fallback("work_enqueue.uid_not_carried");
    }

    let board = match filing_board(&state, &req, &target_agent_uid, (&remote, caller)) {
        Ok(b) => b,
        Err(why) => return err(StatusCode::BAD_REQUEST, why),
    };

    let now = now_ms();
    let ui_change = req.ui_change;
    // Who to tell the filer about if the target can't work now: looked up by the UID it was carried with,
    // else by its name, and named in the note as the filer typed it.
    let target = (if target_agent_uid.is_empty() { req.target_agent.clone() } else { target_agent_uid.clone() }, req.target_agent.clone());
    let item = WorkItem {
        id: Uuid::new_v4().to_string(),
        title: req.title,
        payload: req.payload,
        kind: req.kind,
        target_agent: req.target_agent,
        target_group: req.target_group,
        priority: req.priority,
        // Set by the store: `open`, or `held` when filed on hold (never
        // claimable, not even between the enqueue and a later hold).
        state: if req.hold { crate::backend::storage::work_queue::work_state::HELD.into() } else { String::new() },
        claimed_by: String::new(),
        target_agent_uid,
        claimed_by_uid: String::new(),
        claim_expires: None,
        attempts: 0,
        max_attempts: req.max_attempts.unwrap_or(3),
        created_by: req.created_by,
        // Identity M4c-1 (§6.5.9): the enqueuer's UID is the request's
        // `Caller` — its token — never a body field; `""` when Unattributed
        // (and for an agent of another machine, which has no UID here).
        created_by_uid: super::caller::attributed_uid(caller),
        created_at: now,
        updated_at: now,
        not_before: req.not_before,
        result: String::new(),
    };

    let after = match super::work_tasks::resolve_after(&state, &req.after) {
        Ok(a) => a,
        Err(why) => return err(StatusCode::BAD_REQUEST, why),
    };
    if let Err(why) = after_on_board(&state, &after, &board) {
        return err(StatusCode::BAD_REQUEST, why);
    }
    let first = match req.first.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) if crate::backend::kanban::is_research_kind(f) => Some(f.to_ascii_lowercase()),
        Some(_) => return err(StatusCode::BAD_REQUEST, "first: \"spec\" or \"research\""),
        None => None,
    };
    let filing = super::work_tasks::Filing { board: board.id.clone(), repo: req.repo, review: review_for(req.review, first.is_some()) };
    let card = match &remote {
        Some(actor) => super::work_tasks::remote_card(actor, filing),
        None => super::work_tasks::prepare_card(&state, caller, &item.created_by, filing).await,
    };
    let Some(first) = first else {
        return match enqueue_one(&state, &item, &after, &card) {
            Ok(num) => {
                let _ = state.work_store.work_ui_set(&item.id, ui_change, None, 0, now);
                publish_changed(&state);
                let note = super::availability_view::enqueue_note(&state, &target.0, &target.1).await;
                let body = json!({ "id": item.id, "num": num, "ref": num.map(|n| board.task_ref(n)), "board": board.slug });
                (StatusCode::OK, Json(super::availability_view::with_note(body, note)))
            }
            Err(e) => e,
        };
    };
    file_pair(&state, item, first, &after, &card, (&board, ui_change, now), &target).await
}

/// Which board a filing goes to, if its filer sees it and the target agent is on its list. An agent
/// of another machine (`slug@machine`) is matched against the board's agent list by that name.
fn filing_board(
    state: &AppState,
    req: &EnqueueRequest,
    target_agent_uid: &str,
    (remote, caller): (&Option<String>, Option<&super::caller::Caller>),
) -> Result<crate::backend::storage::work_boards::Board, String> {
    let who: Option<(String, String)> = match (remote, caller) {
        (Some(actor), _) => Some((actor.clone(), String::new())),
        (None, c) => super::work_tasks::viewer(state, c),
    };
    let who_ref = who.as_ref().map(|(a, u)| (a.as_str(), u.as_str()));
    let board = super::work_tasks::pick_board(state, req.board.as_deref(), who_ref, true)?;
    if (!req.target_agent.trim().is_empty() || !target_agent_uid.is_empty()) && !board.visible_to(req.target_agent.trim(), target_agent_uid) {
        return Err(format!("{} isn't on the agent list of board {}; add it there, or file on another board", req.target_agent.trim(), board.slug));
    }
    Ok(board)
}

/// Dependencies stay inside one board.
fn after_on_board(state: &AppState, after: &[String], board: &crate::backend::storage::work_boards::Board) -> Result<(), String> {
    for a in after {
        if let Ok(Some((other, n))) = state.work_store.work_task_board(a) {
            if other.id != board.id {
                return Err(format!(
                    "after: {} is on board {}; a task can only start after tasks on its own board ({})",
                    other.task_ref(n),
                    other.slug,
                    board.slug
                ));
            }
        }
    }
    Ok(())
}

/// A feature as two tasks: its spec (or research), and a build that starts after it, for the same
/// agent, filed and numbered together.
async fn file_pair(
    state: &AppState,
    item: WorkItem,
    first: String,
    after: &[String],
    card: &crate::backend::storage::work_board::NewCard,
    (board, ui_change, now): (&crate::backend::storage::work_boards::Board, Option<bool>, i64),
    target: &(String, String),
) -> (StatusCode, Json<Value>) {
    use crate::backend::kanban::{build_payload, pair_title, spec_payload};
    let spec = WorkItem {
        title: pair_title(&first, &item.title),
        payload: spec_payload(&first, &item.payload),
        kind: first.clone(),
        ..item.clone()
    };
    let build = WorkItem {
        id: Uuid::new_v4().to_string(),
        title: pair_title("build", &item.title),
        payload: build_payload(&item.payload),
        kind: "build".into(),
        ..item
    };
    let spec_num = match enqueue_one(state, &spec, after, card) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let build_num = match enqueue_one(state, &build, std::slice::from_ref(&spec.id), card) {
        Ok(n) => {
            // The flag is the build's: the spec is a document.
            let _ = state.work_store.work_ui_set(&build.id, ui_change, None, 0, now);
            n
        }
        Err(e) => {
            publish_changed(state);
            return e;
        }
    };
    publish_changed(state);
    let note = super::availability_view::enqueue_note(state, &target.0, &target.1).await;
    let body = json!({
        "id": spec.id, "num": spec_num, "ref": spec_num.map(|n| board.task_ref(n)),
        "build_id": build.id, "build_num": build_num, "build_ref": build_num.map(|n| board.task_ref(n)),
        "board": board.slug,
    });
    (StatusCode::OK, Json(super::availability_view::with_note(body, note)))
}

/// The answer to a claim: the attempt (the fence), the card's number and the
/// item, and `after`, the tasks it starts after with their documents. A build
/// claimed by an agent no unlock note reached (it was filed for anyone) finds
/// its spec's path here.
fn claimed_body(state: &AppState, item: &WorkItem, num: Option<i64>) -> Value {
    let after = state.work_store.work_deps_of(&item.id).unwrap_or_default();
    let (task_ref, board) = super::work_tasks::ref_of(state, &item.id).unzip();
    let mut body = json!({
        "claimed": true, "attempt": item.attempts, "num": num, "ref": task_ref, "board": board, "item": item, "after": after,
        "status": "Your status line now shows this task's title. Say what you are doing in your own words with \
                   Presence status (line), and again when your plan changes, when you are blocked or waiting, \
                   and when you are done.",
    });
    if item.target_agent.is_empty() && item.target_agent_uid.is_empty() && item.target_group.is_empty() {
        body["unaimed"] = json!(
            "This task was not aimed at you. If it is outside your area, hand it back with Tasks release (id, attempt, \
             handback: true, reason): it returns to the pool without using an attempt, and an unnamed claim will not \
             offer it to you again."
        );
    }
    body
}

/// The answer to a claim that took: the body above, after the claim's title is written to the agent's
/// status line.
fn claimed_reply(state: &AppState, item: &WorkItem, num: Option<i64>, block_id: &str, holder: (&str, &str)) -> (StatusCode, Json<Value>) {
    super::agent_status_handlers::task_claimed(state, block_id, holder, &item.id, num, &item.title);
    (StatusCode::OK, Json(claimed_body(state, item, num)))
}

/// Whether a filing is held for the operator's Accept: what the filer said, else
/// (a spec-and-build pair) no, as in the operator's "Spec first". Left to the
/// provenance default, a spec an operator asked for would wait for an Accept
/// that never comes, and its build could not start.
fn review_for(asked: Option<bool>, pair: bool) -> Option<bool> {
    asked.or(pair.then_some(false))
}

/// Enqueue one task with what it starts after (the row and the list in one
/// transaction, so no claim takes it in between) and file its card.
fn enqueue_one(
    state: &AppState,
    item: &WorkItem,
    after: &[String],
    card: &crate::backend::storage::work_board::NewCard,
) -> Result<Option<i64>, (StatusCode, Json<Value>)> {
    let enqueued = if after.is_empty() {
        state.work_store.work_queue_enqueue(item).map(|()| None)
    } else {
        state.work_store.work_queue_enqueue_after(item, after)
    };
    match enqueued {
        Ok(Some(why)) => Err(err(StatusCode::BAD_REQUEST, format!("after: {why}"))),
        Ok(None) => Ok(super::work_tasks::file_card(state, item, card)),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, format!("enqueue failed: {e}"))),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaimRequest {
    /// The claiming agent's id.
    pub agent_id: String,
    /// The claiming agent's UID, carried from its own `AGENTMUX_AGENT_UID`
    /// (identity M1a). Optional: a pre-M1a spawn, a continuation resume, an
    /// App Server agent or a quick-launch pane has none. Since M3 it decides
    /// eligibility for UID-addressed items, and is written to
    /// `claimed_by_uid` — since M4c-1 only when the claim is Unattributed;
    /// an attributed claim uses its token's UID instead. It is an assertion
    /// like `agent_id` is — recorded, not trusted.
    #[serde(default)]
    pub agent_uid: String,
    /// The claimer's block, carried from its `AGENTMUX_BLOCKID`. When no UID
    /// was carried, the UID is taken from the row shown on this block — the
    /// identity the block already has, never one derived from `agent_id`.
    #[serde(default)]
    pub block_id: String,
    #[serde(default)]
    pub kind: Option<String>,
    /// Group ids this agent belongs to. Resolved by the CALLER — group
    /// membership lives in the per-channel store, which the queue module
    /// deliberately does not reach into.
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub lease_ms: Option<i64>,
    /// Claim this task (its number or id) and no other: open, or held, which
    /// lifts the hold (SPEC_KANBAN_AGENTS_HOLD_AND_UNHOLD_2026_10_22.md §3.3).
    #[serde(default)]
    pub id: Option<String>,
    /// Claim only on this board (name or slug); without it, on any board the claimer sees, the
    /// default board first on a tie. `"all"` is the same as leaving it out.
    #[serde(default)]
    pub board: Option<String>,
}

/// An Unattributed claimer's UID: carried, else its block's row, else `""`
/// (name path only). An attributed claim uses its `Caller`'s UID instead —
/// see `handle_work_claim`.
async fn claimer_uid(state: &AppState, req: &ClaimRequest) -> String {
    let carried = req.agent_uid.trim();
    if !carried.is_empty() {
        return carried.to_string();
    }
    crate::backend::agent_resolve::record_uid_fallback("work_claim.uid_not_carried");
    let block_id = req.block_id.trim().to_string();
    if block_id.is_empty() {
        return String::new();
    }
    // A synchronous store read, so off the async worker (incident #1782).
    let mstore = state.mstore.clone();
    tokio::task::spawn_blocking(move || {
        crate::backend::agent_resolve::uid_for_block(&mstore, &block_id)
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

pub(super) async fn handle_work_claim(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<ClaimRequest>,
) -> (StatusCode, Json<Value>) {
    super::actor::check_actor(
        &state,
        caller.as_deref(),
        super::actor::ActorSite::WorkClaim,
        Some(&req.agent_id),
    );
    super::actor::check_carried_uid(caller.as_deref(), &req.agent_uid);
    if req.agent_id.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "agent_id is required");
    }
    let now = now_ms();

    // Reap BEFORE claiming, so an expired lease is recovered by the very next
    // agent that asks for work. This is why there is no background reaper
    // task: the only moment a stale claim actually matters is when someone
    // wants the item, and reaping here makes that self-healing. The predicate
    // is indexed (idx_ids_work_queue_claim_expires) and no-ops when nothing
    // has expired.
    // A holder srv knows is alive keeps its lease whether or not it calls
    // heartbeat (Kanban §6.8); what still expires is recorded as such.
    super::work_tasks::extend_live_leases(&state, now);
    let expiring = super::work_tasks::expiring(&state, now);
    if let Err(e) = state.work_store.work_queue_reap(now) {
        // Not fatal — a claim against a not-yet-reaped queue is still correct,
        // just possibly emptier than it could be. Log and continue rather than
        // failing the caller's request.
        tracing::warn!(target: "workqueue", error = %e, "reap before claim failed");
    }
    for gone in expiring {
        let to = state.work_store.work_queue_get(&gone.id).ok().flatten().map(|i| i.state).unwrap_or_default();
        if to != crate::backend::storage::work_queue::work_state::CLAIMED {
            super::work_tasks::record(&state, &gone.id, &gone.claimed_by, "system", "expired", "claimed", &to, "lease expired");
        }
    }

    // Identity M4c-1 (§6.5.9): an attributed claim's UID is the request's
    // `Caller` — its token. Only an Unattributed claim falls back to M3's
    // path: the UID it CARRIED, else the UID of the row on its own block
    // (the presence path's resolution, §0.1) — never one derived from its
    // name (§2 rule 1). Deriving it from the name reached the host-wide slug
    // registry, so a claimer with no UID in its env could be locked out of
    // its own UID-addressed work, or handed another channel's (review on
    // #3563). Each fallback is counted (§9.2): a non-zero count names a
    // launch path that does not carry its UID. This one UID is both the
    // `claimed_by_uid` written and the M3 eligibility match
    // (`target_agent_uid = ?5`) — the one reader change M4c-1 makes; the
    // carried UID and the token are set together from one row, and a
    // disagreement is counted just above (`check_carried_uid`).
    let agent_uid = match caller.as_deref().and_then(super::caller::Caller::uid) {
        Some(uid) => uid.to_string(),
        None => claimer_uid(&state, &req).await,
    };

    let filter = ClaimFilter {
        kind: req.kind.filter(|k| !k.is_empty()),
        agent_id: req.agent_id.clone(),
        agent_uid,
        groups: req.groups.clone(),
    };
    let lease = req.lease_ms.filter(|&n| n > 0).unwrap_or(DEFAULT_LEASE_MS);

    let on_board = match req.board.as_deref().map(str::trim).filter(|b| !b.is_empty() && !b.eq_ignore_ascii_case("all")) {
        Some(key) => match super::work_tasks::pick_board(&state, Some(key), Some((req.agent_id.as_str(), filter.agent_uid.as_str())), false) {
            Ok(b) => Some(b),
            Err(why) => return err(StatusCode::BAD_REQUEST, why),
        },
        None => None,
    };

    if let Some(named) = req.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // With a board named, a bare number is that board's.
        let named = match &on_board {
            Some(b) if named.trim_start_matches('#').chars().all(|c| c.is_ascii_digit()) => format!("{}#{}", b.slug, named.trim_start_matches('#')),
            _ => named.to_string(),
        };
        let id = super::work_tasks::resolve_id(&state, &named);
        if let Some(why) = unclaimable_board(&state, &id, &named, &filter) {
            return (StatusCode::OK, Json(json!({ "claimed": false, "why": why })));
        }
        return match state.work_store.work_claim_one(&id, &filter, now, lease) {
            Ok(Some((item, from))) => {
                let num = state.work_store.work_card_ensure(&item.id, now).ok();
                if from == crate::backend::storage::work_queue::work_state::HELD {
                    super::work_tasks::record(&state, &item.id, &item.claimed_by, "agent", "unheld", "held", "open", "claimed by name");
                }
                super::work_tasks::record(&state, &item.id, &item.claimed_by, "agent", "claimed", "open", "claimed", "");
                publish_changed(&state);
                claimed_reply(&state, &item, num, &req.block_id, (&req.agent_id, holder_uid(caller.as_deref())))
            }
            Ok(None) => {
                let why = super::work_tasks::why_not_claimed(&state, &id, &filter, now);
                (StatusCode::OK, Json(json!({ "claimed": false, "why": why })))
            }
            Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("claim failed: {e}")),
        };
    }

    let floor = crate::backend::quota_mode::claim_floor(&state.config_watcher.get_full_config().settings.extra);
    match state.work_store.work_queue_claim_in(&filter, on_board.as_ref().map(|b| b.id.as_str()), now, lease, floor) {
        Ok(Some(item)) => {
            let num = state.work_store.work_card_ensure(&item.id, now).ok();
            super::work_tasks::record(&state, &item.id, &item.claimed_by, "agent", "claimed", "open", "claimed", "");
            publish_changed(&state);
            // `attempt` is echoed at the top level as well as inside `item`
            // because every subsequent call (heartbeat/complete/release) MUST
            // pass it back as a fence — see work_queue.rs's ABA note. Making
            // it prominent here is the difference between a caller that
            // fences correctly and one that discovers the requirement from a
            // 404-shaped "not the holder" response.
            claimed_reply(&state, &item, num, &req.block_id, (&req.agent_id, holder_uid(caller.as_deref())))
        }
        Ok(None) => nothing_to_claim(&state, &filter, now, floor),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("claim failed: {e}")),
    }
}

/// Why a named task can't be claimed because of its board: one the claimer doesn't see is as good
/// as unknown, and an archived board's tasks are read-only.
fn unclaimable_board(state: &AppState, id: &str, named: &str, filter: &ClaimFilter) -> Option<String> {
    let Ok(Some((b, _))) = state.work_store.work_task_board(id) else { return None };
    if !b.visible_to(&filter.agent_id, &filter.agent_uid) {
        return Some(format!("there is no task {named}"));
    }
    b.archived.then(|| format!("board {} is archived: its tasks can't be claimed", b.slug))
}

/// The answer when an unnamed claim finds nothing. Only when the quota floor is what stood in the
/// way does it say so.
fn nothing_to_claim(state: &AppState, filter: &ClaimFilter, now: i64, floor: Option<i64>) -> (StatusCode, Json<Value>) {
    let held = floor.and_then(|n| state.work_store.work_queue_held_by_floor(filter, now, n).ok().filter(|&h| h > 0).map(|h| (n, h)));
    match held {
        Some((n, held)) => (
            StatusCode::OK,
            Json(json!({
                "claimed": false,
                "held_back_by_quota_mode": held,
                "reason": format!(
                    "quota mode: {held} open task(s) held back; an unnamed claim takes only tasks aimed at you and ones with priority {n} or more. Claim a named one with its id if you were told to"
                ),
            })),
        ),
        None => (StatusCode::OK, Json(json!({ "claimed": false }))),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct HolderRequest {
    pub agent_id: String,
    /// Fence token — the `attempt` returned by the claim this call belongs to.
    pub attempt: i64,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub lease_ms: Option<i64>,
    /// release: the task is outside the holder's area. It goes back to the pool with its attempt
    /// given back (never parked as failed) and is not offered to this agent by an unnamed claim again.
    #[serde(default)]
    pub handback: bool,
}

/// The caller's UID for a holder check (identity M4c-2, spec §6.5.9): the
/// token's, or `""` when Unattributed — the store then matches by name.
fn holder_uid(caller: Option<&super::caller::Caller>) -> &str {
    caller.and_then(super::caller::Caller::uid).unwrap_or("")
}

/// Count a holder check that matched by name — the row or the caller had no
/// UID. Each is a holder that M5, which drops the name path, would refuse.
fn count_holder_match(matched: Option<HolderMatch>) {
    if matched == Some(HolderMatch::Name) {
        crate::backend::agent_resolve::record_uid_fallback("m4c.holder_by_name");
    }
}

/// Shared shape for the three holder-only transitions. No match from the
/// store means the caller is not the current holder OR its fence is stale — both are
/// CONFLICT, not NOT_FOUND: the row usually still exists, it just moved on
/// without this caller.
fn holder_result(ok: bool, state: &AppState) -> (StatusCode, Json<Value>) {
    if ok {
        publish_changed(state);
        (StatusCode::OK, Json(json!({ "ok": true })))
    } else {
        (
            StatusCode::CONFLICT,
            Json(json!({
                "ok": false,
                "error": "not the current holder, or this claim has been superseded \
                          (expired and reclaimed) — re-claim before continuing"
            })),
        )
    }
}

pub(super) async fn handle_work_heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<HolderRequest>,
) -> (StatusCode, Json<Value>) {
    super::actor::check_actor(
        &state,
        caller.as_deref(),
        super::actor::ActorSite::WorkHeartbeat,
        Some(&req.agent_id),
    );
    let id = super::work_tasks::resolve_id(&state, &id);
    let lease = req.lease_ms.filter(|&n| n > 0).unwrap_or(DEFAULT_LEASE_MS);
    match state
        .work_store
        .work_queue_heartbeat(&id, &req.agent_id, holder_uid(caller.as_deref()), req.attempt, now_ms(), lease)
    {
        Ok(matched) => {
            count_holder_match(matched);
            holder_result(matched.is_some(), &state)
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("heartbeat failed: {e}")),
    }
}

pub(super) async fn handle_work_complete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<HolderRequest>,
) -> (StatusCode, Json<Value>) {
    super::actor::check_actor(
        &state,
        caller.as_deref(),
        super::actor::ActorSite::WorkComplete,
        Some(&req.agent_id),
    );
    let id = super::work_tasks::resolve_id(&state, &id);
    // A task that needs review goes to Review instead of done (the agent
    // can't accept its own work); either way the answer names the agent's
    // next waiting task (Kanban §6.5).
    let done = super::work_tasks::finish(&state, caller.as_deref(), &id, &req.agent_id, req.attempt, &req.result, None).await;
    if done.0.is_success() {
        // The agent's build caches are cleared in the background (SPEC_BUILD_CACHE_CLEANUP_2026_10_26.md).
        crate::backend::build_cache_service::agent_task_done(state.mstore.clone(), state.config_watcher.clone(), req.agent_id.clone());
    }
    done
}

/// Returns the item's RESULTING state alongside `ok` (Codex P2 on PR #2902).
/// A release on the item's final allowed attempt parks it as `failed` rather
/// than reopening it, so a bare `{ok:true}` would let the caller report "handed
/// back to the queue" for an item nobody can ever claim again.
pub(super) async fn handle_work_release(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Json(req): Json<HolderRequest>,
) -> (StatusCode, Json<Value>) {
    super::actor::check_actor(
        &state,
        caller.as_deref(),
        super::actor::ActorSite::WorkRelease,
        Some(&req.agent_id),
    );
    let id = super::work_tasks::resolve_id(&state, &id);
    let released = if req.handback {
        state.work_store.work_queue_hand_back(&id, &req.agent_id, holder_uid(caller.as_deref()), req.attempt, &req.result, now_ms())
    } else {
        state.work_store.work_queue_release(&id, &req.agent_id, holder_uid(caller.as_deref()), req.attempt, &req.result, now_ms())
    };
    match released {
        Ok(Some(matched)) => {
            count_holder_match(Some(matched));
            publish_changed(&state);
            // Read back rather than infer: the store owns the
            // attempts-vs-max_attempts decision, and duplicating that rule
            // here is exactly how the two drift.
            let resulting = state
                .work_store
                .work_queue_get(&id)
                .ok()
                .flatten()
                .map(|i| i.state)
                .unwrap_or_default();
            // A hand-back's history line was written with the hand-back itself.
            if !req.handback {
                super::work_tasks::record(&state, &id, &req.agent_id, "agent", "released", "claimed", &resulting, &req.result);
            }
            super::agent_status_handlers::task_ended(&state, &req.agent_id, holder_uid(caller.as_deref()), &id);
            (StatusCode::OK, Json(json!({ "ok": true, "state": resulting })))
        }
        Ok(None) => holder_result(false, &state),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("release failed: {e}")),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ListQuery {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub limit: Option<usize>,
    /// A board by name or slug; `all` for every board the caller sees; the default board when absent.
    #[serde(default)]
    pub board: Option<String>,
}

pub(super) async fn handle_work_list(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Query(q): Query<ListQuery>,
) -> (StatusCode, Json<Value>) {
    let who = super::work_tasks::viewer(&state, caller.as_deref());
    let who = who.as_ref().map(|(a, u)| (a.as_str(), u.as_str()));
    match list_view(&state, &q.state, q.limit, q.board.as_deref(), who) {
        Ok(view) => (StatusCode::OK, Json(view)),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

/// The board's tasks in `state` (all when empty), newest first as the store gives them, each with its
/// number and ref, which agents and the operator use. Without `board`, the default board's; `all` lists
/// every board the caller sees. `who` is the caller's id and UID, whose agent lists decide the boards it
/// sees (none: all of them). The answer carries a line when more waits for the caller on other boards.
pub(super) fn list_view(
    state: &AppState,
    filter: &str,
    limit: Option<usize>,
    board: Option<&str>,
    who: Option<(&str, &str)>,
) -> Result<Value, String> {
    let limit = limit.unwrap_or(50).clamp(1, 500);
    let now = now_ms();
    let key = board.map(str::trim).filter(|b| !b.is_empty());
    let all = key.is_some_and(|k| k.eq_ignore_ascii_case("all"));
    let (ids, scope, scope_id) = if all {
        let boards = state.work_store.work_boards_list(true, now).map_err(|e| format!("list failed: {e}"))?;
        let ids: Vec<String> = boards.into_iter().filter(|b| who.map_or(true, |(a, u)| b.visible_to(a, u))).map(|b| b.id).collect();
        (ids, "all".to_string(), String::new())
    } else {
        let b = super::work_tasks::pick_board(state, key, who, false)?;
        (vec![b.id.clone()], b.slug.clone(), b.id)
    };
    let items = state.work_store.work_queue_list_on(filter, limit, &ids, now).map_err(|e| format!("list failed: {e}"))?;
    let items: Vec<Value> = items
        .into_iter()
        .map(|i| {
            let num = state.work_store.work_card_ensure(&i.id, now).ok();
            let (task_ref, board) = super::work_tasks::ref_of(state, &i.id).unzip();
            let mut v = serde_json::to_value(&i).unwrap_or_default();
            if let Some(o) = v.as_object_mut() {
                o.insert("num".into(), json!(num));
                o.insert("ref".into(), json!(task_ref));
                o.insert("board".into(), json!(board));
            }
            v
        })
        .collect();
    let mut view = json!({ "items": items, "board": scope });
    if let (false, Some((agent, uid))) = (all, who) {
        let elsewhere = state.work_store.work_waiting_elsewhere(&scope_id, agent, uid).unwrap_or_default();
        let total: i64 = elsewhere.iter().map(|(_, n)| n).sum();
        if total > 0 {
            let names: Vec<&str> = elsewhere.iter().map(|(name, _)| name.as_str()).collect();
            view["note"] = json!(format!("{total} more for you on {} (list with board: \"all\", or name one)", names.join(" and ")));
        }
    }
    Ok(view)
}

/// [`list_view`]'s tasks alone, for the remote `tasks.list`.
pub(super) fn list_items(
    state: &AppState,
    filter: &str,
    limit: Option<usize>,
    board: Option<&str>,
    who: Option<(&str, &str)>,
) -> Result<Vec<Value>, String> {
    let view = list_view(state, filter, limit, board, who)?;
    Ok(view["items"].as_array().cloned().unwrap_or_default())
}

#[cfg(test)]
mod review_for_tests {
    use super::review_for;

    #[test]
    fn a_pair_defaults_to_no_accept_unless_asked() {
        assert_eq!(review_for(None, true), Some(false));
        assert_eq!(review_for(Some(true), true), Some(true), "asked for stands");
        assert_eq!(review_for(None, false), None, "a single task keeps the provenance default");
        assert_eq!(review_for(Some(false), false), Some(false));
    }
}
