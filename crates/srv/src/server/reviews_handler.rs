// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/reviews`: every pull request a Kanban task names, with the review, checks and merge state this srv
//! was told of and what it waits on (`backend::reviews`, spec `SPEC_REVIEWS_PANE_2026_11_15.md`). It reads the board
//! and the in-memory recent events and calls no GitHub API.
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
use crate::backend::kanban::{prs, FINISHED_WINDOW_MS};
use crate::backend::reviews::{self, CardIn, ReviewRow, ReviewsList, ReviewsTask};

/// The most tasks read from the board.
const TASKS_LIMIT: usize = 500;

#[derive(Debug, Default, Deserialize)]
pub struct ReviewsQuery {
    pub agent: Option<String>,
    pub waits_on: Option<String>,
    pub task: Option<String>,
    pub repo: Option<String>,
}

/// The cards of `tasks` that name a pull request, as [`reviews::build`] reads them.
fn cards_of(tasks: &[crate::backend::storage::work_board::BoardTask]) -> Vec<CardIn> {
    tasks
        .iter()
        .filter_map(|t| {
            let pr = prs::parse_pr_ref(&t.card.pr_url, &t.card.repo)?;
            let holder = if t.item.claimed_by.is_empty() { &t.item.target_agent } else { &t.item.claimed_by };
            let (merged_at_ms, released_in) = t.card.pr.as_ref().map_or((None, String::new()), |p| (p.merged_at, p.released_in.clone()));
            Some(CardIn {
                task: ReviewsTask {
                    task_ref: if t.card.task_ref.is_empty() { format!("#{}", t.card.num) } else { t.card.task_ref.clone() },
                    num: t.card.num,
                    title: t.item.title.clone(),
                    state: t.item.state.clone(),
                },
                agent: holder.to_ascii_lowercase(),
                repo: pr.repo,
                number: pr.number,
                branch: t.card.branch.clone(),
                merged_at_ms,
                released_in,
                updated_at_ms: t.card.updated_at,
            })
        })
        .collect()
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
        // A card with no board yet is on the default one.
        let default_seen = boards.iter().any(|b| b.is_default && who.as_ref().map_or(true, |(agent, uid)| b.visible_to(agent, uid)));
        let visible: HashSet<String> =
            boards.into_iter().filter(|b| who.as_ref().map_or(true, |(agent, uid)| b.visible_to(agent, uid))).map(|b| b.id).collect();
        let tasks: Vec<_> = work
            .work_board_list_in(None, now - FINISHED_WINDOW_MS, TASKS_LIMIT, now)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| who.is_none() || visible.contains(&t.card.board_id) || (t.card.board_id.is_empty() && default_seen))
            .collect();
        let mut list = reviews::build(&cards_of(&tasks), &crate::backend::pr_flow::recent(), now);
        if let Some((agent, _)) = &who {
            // A PR no visible task holds is the caller's only when the notice went to it.
            list.rows.retain(|r| r.task.is_some() || r.agent.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(agent)));
        }
        list
    })
    .await
    .unwrap_or_default();
    Json(narrow(list, &q))
}

#[cfg(test)]
#[path = "reviews_handler_tests.rs"]
mod tests;
