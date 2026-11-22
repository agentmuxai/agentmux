// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/reviews`: every pull request a Kanban task names, with the review, checks and merge state this srv
//! was told of and what it waits on (`backend::reviews`, spec `SPEC_REVIEWS_PANE_2026_11_15.md`). A thin route over the
//! rows the board shows on a card (the card store keeps what each notice said, across restarts), plus the pull
//! requests only a recent notice named. It calls no GitHub API.
//!
//! A caller that is an agent gets the tasks of the boards `Tasks list` shows it and no others, and a PR only an event
//! named only when the notice went to that agent: a private repository's PR is never given to a caller that can't
//! see its task. A caller with no token (the app's own windows) gets everything.
//!
//! Query: `agent` (the task's holder or the notified agent), `waits_on` (`person`, `author`, ...), `task` (`12`,
//! `#12`, `work#12`) and `repo` (`owner/repo`), each narrowing the list.

use std::collections::HashSet;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use super::AppState;
use crate::backend::kanban::FINISHED_WINDOW_MS;
use crate::backend::pr_flow::PrFlowBy;
use crate::backend::reviews::{self, ReviewRow, ReviewsChecks, ReviewsList, ReviewsTask, ReviewsVerdict, WaitsOn};
use crate::backend::storage::work_board::BoardTask;

/// The most tasks read from one board. Each board is read on its own, so a busy board cannot push another's
/// tasks out of the cut, and a caller's visible boards are the only ones read.
const TASKS_LIMIT: usize = 500;

#[derive(Debug, Default, Deserialize)]
pub struct ReviewsQuery {
    pub agent: Option<String>,
    pub waits_on: Option<String>,
    pub task: Option<String>,
    pub repo: Option<String>,
}

/// A row for each pull request the cards of `tasks` hold, from what the card store keeps of it (the review, checks and
/// merge notices it was told of and what it waits on): the same rows the board shows on a card.
fn card_rows(tasks: &[BoardTask]) -> Vec<ReviewRow> {
    let by = |name: &str| match name {
        "bot" => PrFlowBy::Bot,
        "codex" => PrFlowBy::Codex,
        _ => PrFlowBy::Person,
    };
    let head = |h: &str| (!h.is_empty()).then(|| h.to_string());
    let mut rows = Vec::new();
    for t in tasks {
        let holder = if t.item.claimed_by.is_empty() { &t.item.target_agent } else { &t.item.claimed_by };
        let task_ref = if t.card.task_ref.is_empty() { format!("#{}", t.card.num) } else { t.card.task_ref.clone() };
        let released_in = t.card.pr.as_ref().map(|p| p.released_in.clone()).unwrap_or_default();
        for p in &t.card.prs {
            let verdicts: Vec<ReviewsVerdict> = p
                .verdicts
                .iter()
                .map(|v| ReviewsVerdict { by: by(&v.by), state: v.state.clone(), head: head(&v.head), at_ms: v.at.max(0) as u64, stale: v.stale })
                .collect();
            let checks = (p.checks != "none").then(|| ReviewsChecks { state: p.checks.clone(), head: head(&p.checks_head), at_ms: p.checks_at.max(0) as u64 });
            let changed = verdicts.iter().map(|v| v.at_ms as i64).chain([p.checks_at, p.flow_at, p.updated_at, t.card.updated_at]).max().unwrap_or(0);
            rows.push(ReviewRow {
                repo: Some(p.repo.clone()),
                repo_id: p.repo_id.map(|id| id as u64),
                number: p.number,
                url: Some(p.url.clone()),
                task: Some(ReviewsTask { task_ref: task_ref.clone(), num: t.card.num, title: t.item.title.clone(), state: t.item.state.clone() }),
                agent: Some(holder.to_ascii_lowercase()).filter(|a| !a.is_empty()),
                branch: t.card.branch.clone(),
                reviews: verdicts,
                checks,
                merge: p.merge.clone(),
                merged_at_ms: p.merged_at,
                released_in: released_in.clone(),
                waits_on: WaitsOn::parse(&p.waits_on),
                why: p.why.clone(),
                updated_at_ms: changed,
            });
        }
    }
    rows
}

/// Whether `task` names `row`'s task. Task numbers are per board, so a bare `12` or `#12` is the default board's
/// task 12 and another board's is `work#12`.
fn is_task(row: &ReviewRow, task: &str) -> bool {
    let want = task.trim().to_ascii_lowercase();
    let want = if !want.is_empty() && want.trim_start_matches('#').bytes().all(|b| b.is_ascii_digit()) { format!("#{}", want.trim_start_matches('#')) } else { want };
    row.task.as_ref().is_some_and(|t| t.task_ref.to_ascii_lowercase() == want)
}

/// Narrows `list` to what `q` asks for.
fn narrow(mut list: ReviewsList, q: &ReviewsQuery) -> ReviewsList {
    let lower = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_ascii_lowercase);
    let (agent, repo) = (lower(&q.agent), lower(&q.repo));
    let waits = lower(&q.waits_on);
    let task = q.task.as_deref().map(str::trim).filter(|s| !s.is_empty());
    list.rows.retain(|r| {
        agent.as_ref().map_or(true, |a| r.agent.as_deref().map(str::to_ascii_lowercase).as_ref() == Some(a))
            && repo.as_ref().map_or(true, |p| r.repo.as_deref().map(str::to_ascii_lowercase).as_ref() == Some(p))
            && waits.as_ref().map_or(true, |w| serde_json::to_value(r.waits_on).ok().and_then(|v| v.as_str().map(str::to_string)).as_deref() == Some(w.as_str()))
            && task.map_or(true, |t| is_task(r, t))
    });
    list
}

/// The rows of the cards, and after them the pull requests only a notice named (`orphans`: a list built from the
/// notices alone), newest change first. A notice whose repository id a card settled is that card's, not a row of its
/// own; a pull request no visible task holds is the caller's only when the notice went to it.
fn join(mut rows: Vec<ReviewRow>, orphans: ReviewsList, caller: Option<&str>) -> ReviewsList {
    let held: HashSet<(u64, u64)> = rows.iter().filter_map(|r| r.repo_id.map(|id| (id, r.number))).collect();
    rows.extend(orphans.rows.into_iter().filter(|r| {
        let settled = r.repo_id.is_some_and(|id| held.contains(&(id, r.number)));
        !settled && caller.map_or(true, |c| r.agent.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(c)))
    }));
    rows.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then(a.number.cmp(&b.number)));
    ReviewsList { rows, as_of_ms: orphans.as_of_ms, last_notice_at_ms: orphans.last_notice_at_ms }
}

pub async fn handle_reviews(
    State(state): State<AppState>,
    caller: Option<axum::Extension<super::caller::Caller>>,
    Query(q): Query<ReviewsQuery>,
) -> impl IntoResponse {
    let who = super::work_tasks::viewer(&state, caller.as_deref());
    let work = state.work_store.clone();
    let now = agentmux_common::time::now_ms();
    let list = tokio::task::spawn_blocking(move || {
        let boards = work.work_boards_list(true, now).unwrap_or_default();
        let seen: Vec<_> = boards.into_iter().filter(|b| who.as_ref().map_or(true, |(agent, uid)| b.visible_to(agent, uid))).collect();
        let mut tasks: Vec<_> = Vec::new();
        let mut read: HashSet<String> = HashSet::new();
        for board in &seen {
            if read.insert(board.id.clone()) {
                tasks.extend(work.work_board_list_in(Some(board.id.as_str()), now - FINISHED_WINDOW_MS, TASKS_LIMIT, now).unwrap_or_default());
            }
        }
        join(card_rows(&tasks), reviews::build(&[], &crate::backend::pr_flow::recent(), now), who.as_ref().map(|(agent, _)| agent.as_str()))
    })
    .await
    .unwrap_or_default();
    Json(narrow(list, &q))
}

#[cfg(test)]
#[path = "reviews_handler_tests.rs"]
mod tests;
