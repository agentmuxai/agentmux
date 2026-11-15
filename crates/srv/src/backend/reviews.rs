// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Reviews pane's list (`GET /api/v1/reviews`, spec `SPEC_REVIEWS_PANE_2026_11_15.md`): every pull request a
//! Kanban task names, with the review, checks and merge state srv was told of, and what each one waits on.
//!
//! Phase R1 makes no GitHub call and holds no credential. It joins two things srv already has: the board's cards
//! (their PR address, repository, branch, holder and merge time) and the recent review, checks and merge events
//! of [`super::pr_flow`]. The events name a repository only by its numeric id, so the join is by the card's
//! holder and the PR number, and a repository id learned from one unambiguous match settles a later clash of two
//! cards that hold PRs with the same number. An event no card claims is a row with no task and no address.
//!
//! This file is pure: the handler reads the board and the recent events and calls [`build`].

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use super::pr_flow::{ChecksFlowEvent, MergeFlowEvent, PrFlowBy, PrFlowSnapshot, ReviewFlowEvent};

/// The task a pull request belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReviewsTask {
    /// How a message names it: `#12`, or `work#12` off the default board.
    #[serde(rename = "ref")]
    pub task_ref: String,
    #[ts(type = "number")]
    pub num: i64,
    pub title: String,
    /// The task's own state: `open`, `claimed`, `review`, `done`, ...
    pub state: String,
}

/// A card that names a pull request, as [`build`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardIn {
    pub task: ReviewsTask,
    /// The agent that holds (or last held) the task, lower case; empty when none.
    pub agent: String,
    /// `owner/repo`, lower case.
    pub repo: String,
    pub number: u64,
    pub branch: String,
    pub merged_at_ms: Option<i64>,
    pub released_in: String,
    pub updated_at_ms: i64,
}

/// One reviewer kind's latest verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReviewsVerdict {
    pub by: PrFlowBy,
    /// `approved`, `changes_requested`, `commented`, or another well-formed name.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
    #[ts(type = "number")]
    pub at_ms: u64,
    /// The pull request's head moved after this verdict: it was given on an older commit.
    pub stale: bool,
}

/// The checks of the PR's newest head srv heard of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReviewsChecks {
    /// `passing`, `failing`, or another well-formed name (`pending`).
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
    #[ts(type = "number")]
    pub at_ms: u64,
}

/// Who has to act next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "lowercase")]
pub enum WaitsOn {
    /// A decision only a person makes.
    Person,
    /// The agent that wrote it.
    Author,
    /// A reviewer: no verdict on the current head.
    Reviewer,
    /// A required check that is still running.
    Ci,
    /// The merge queue.
    Queue,
    /// Nothing: merged or closed.
    None,
    /// The rule could not place it.
    Unknown,
}

/// A pull request and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReviewRow {
    /// `owner/repo`; none for a PR only an event named.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub repo: Option<String>,
    /// GitHub's numeric repository id, when an event carried it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub repo_id: Option<u64>,
    #[ts(type = "number")]
    pub number: u64,
    /// Where it is on the web; none when the repository is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub task: Option<ReviewsTask>,
    /// The agent it is for (the task's holder, else the one the notices went to).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent: Option<String>,
    pub branch: String,
    /// The latest verdict of each reviewer kind.
    pub reviews: Vec<ReviewsVerdict>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub checks: Option<ReviewsChecks>,
    /// `open`, `merged`, or another well-formed name (`closed`).
    pub merge: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub merged_at_ms: Option<i64>,
    /// The release that holds the merge, when the task's card knows it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub released_in: String,
    pub waits_on: WaitsOn,
    /// A short code for why: `changes_requested`, `checks_failing`, `checks_running`, `no_review`,
    /// `review_stale`, `awaiting_accept`, `ready_to_merge`, `merged`, `closed`, `task_finished`, `queued`, `unknown`.
    pub why: String,
    /// When anything about it last changed, in milliseconds since 1970.
    #[ts(type = "number")]
    pub updated_at_ms: i64,
}

/// The answer of `GET /api/v1/reviews`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReviewsList {
    /// Newest change first.
    pub rows: Vec<ReviewRow>,
    /// When this was built.
    #[ts(type = "number")]
    pub as_of_ms: i64,
    /// When the newest review, checks or merge notice came; none before the first since srv started. A list built
    /// from notices is only as complete as they are: this is how old that knowledge is.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub last_notice_at_ms: Option<u64>,
}

/// What [`waits_on`] reads of one pull request.
pub struct WaitsInput<'a> {
    /// `open`, `merged`, `closed`, `queued`, or another name.
    pub merge: &'a str,
    pub checks: Option<&'a str>,
    /// The latest verdict of each reviewer kind: its state, and whether the head moved since.
    pub reviews: &'a [(&'a str, bool)],
    /// The task's own state.
    pub task_state: Option<&'a str>,
}

/// Who has to act next on a PR, and why. One table; a change of the review system changes this function only.
///
/// A bot is never "the person who acts": `person` is only the task waiting for the operator's accept after the
/// reviewers approved. A PR the table cannot place is `unknown`, never a guess.
pub fn waits_on(i: &WaitsInput<'_>) -> (WaitsOn, &'static str) {
    match i.merge {
        "merged" => return (WaitsOn::None, "merged"),
        "closed" => return (WaitsOn::None, "closed"),
        "queued" => return (WaitsOn::Queue, "queued"),
        _ => {}
    }
    // A task that is over (done, cancelled, failed) with a PR no one saw merge or close waits on no one.
    if matches!(i.task_state, Some("done" | "cancelled" | "failed")) {
        return (WaitsOn::None, "task_finished");
    }
    if i.checks == Some("failing") {
        return (WaitsOn::Author, "checks_failing");
    }
    let fresh = |state: &str| i.reviews.iter().any(|(s, stale)| *s == state && !stale);
    if fresh("changes_requested") {
        return (WaitsOn::Author, "changes_requested");
    }
    if matches!(i.checks, Some(c) if c != "passing") {
        return (WaitsOn::Ci, "checks_running");
    }
    if fresh("approved") {
        return if i.task_state == Some("review") {
            (WaitsOn::Person, "awaiting_accept")
        } else {
            (WaitsOn::Author, "ready_to_merge")
        };
    }
    if i.reviews.is_empty() {
        (WaitsOn::Reviewer, "no_review")
    } else if i.reviews.iter().all(|(_, stale)| *stale) {
        (WaitsOn::Reviewer, "review_stale")
    } else {
        // Only a comment on the current head: no verdict yet.
        (WaitsOn::Reviewer, "no_review")
    }
}

/// The key a pull request has in the events: its repository's id and its number.
type EventKey = (u64, u64);

/// Binds each pull request the events name to the card that holds it. Whether a card is the one: it has the
/// event's agent (the notices go to the PR's own agent) and the PR's number. Two cards of one agent with the
/// same number in different repositories are told apart by a repository id learned from a match with one
/// candidate; with none learned the event stays unbound.
fn bind(cards: &[CardIn], flow: &PrFlowSnapshot) -> HashMap<EventKey, usize> {
    let mut triples: BTreeSet<(String, u64, u64)> = BTreeSet::new();
    let mut add = |agent: &str, repo_id: u64, number: u64| {
        triples.insert((agent.to_ascii_lowercase(), repo_id, number));
    };
    flow.review.iter().for_each(|e| add(&e.agent, e.pr.repo_id, e.pr.number));
    flow.checks.iter().for_each(|e| add(&e.agent, e.pr.repo_id, e.pr.number));
    flow.merge.iter().for_each(|e| add(&e.agent, e.pr.repo_id, e.pr.number));

    let candidates = |agent: &str, number: u64| -> Vec<usize> {
        cards.iter().enumerate().filter(|(_, c)| c.number == number && !c.agent.is_empty() && c.agent == agent).map(|(i, _)| i).collect()
    };
    let mut bound: HashMap<EventKey, usize> = HashMap::new();
    let mut known: HashMap<u64, String> = HashMap::new();
    let mut later: Vec<(String, u64, u64)> = Vec::new();
    for (agent, repo_id, number) in &triples {
        let found = candidates(agent, *number);
        let repos: BTreeSet<&str> = found.iter().map(|&i| cards[i].repo.as_str()).collect();
        if repos.len() == 1 {
            let i = found[0];
            let repo = &cards[i].repo;
            // One repository id is one repository: a card the id does not match is not this PR's card.
            if known.iter().any(|(id, r)| (id == repo_id) != (r == repo)) {
                continue;
            }
            known.insert(*repo_id, repo.clone());
            bound.entry((*repo_id, *number)).or_insert(i);
        } else {
            later.push((agent.clone(), *repo_id, *number));
        }
    }
    for (agent, repo_id, number) in later {
        if bound.contains_key(&(repo_id, number)) {
            continue;
        }
        let Some(repo) = known.get(&repo_id) else { continue };
        // The repository is known: the one card of it with this number (its holder may have changed).
        let by_agent = candidates(&agent, number).into_iter().find(|&i| &cards[i].repo == repo);
        let any = || cards.iter().position(|c| c.number == number && &c.repo == repo);
        if let Some(i) = by_agent.or_else(any) {
            bound.insert((repo_id, number), i);
        }
    }
    bound
}

/// The newest of `events` by time, per `key`.
fn newest<'a, T>(events: &'a [T], at: impl Fn(&T) -> u64) -> Option<&'a T> {
    events.iter().max_by_key(|e| at(e))
}

/// One pull request's events, picked from the snapshot by its key.
struct Events<'a> {
    review: Vec<&'a ReviewFlowEvent>,
    checks: Vec<&'a ChecksFlowEvent>,
    merge: Vec<&'a MergeFlowEvent>,
}

fn events_of<'a>(flow: &'a PrFlowSnapshot, key: EventKey) -> Events<'a> {
    let is = |repo_id: u64, number: u64| (repo_id, number) == key;
    Events {
        review: flow.review.iter().filter(|e| is(e.pr.repo_id, e.pr.number)).collect(),
        checks: flow.checks.iter().filter(|e| is(e.pr.repo_id, e.pr.number)).collect(),
        merge: flow.merge.iter().filter(|e| is(e.pr.repo_id, e.pr.number)).collect(),
    }
}

/// The row for one pull request from its events and, when a task names it, its card.
fn row_of(ev: &Events<'_>, key: Option<EventKey>, card: Option<&CardIn>, orphan_agent: Option<&str>) -> ReviewRow {
    let checks = ev.checks.iter().copied().max_by_key(|e| e.at_ms).map(|e| ReviewsChecks { state: e.state.clone(), head: e.head.clone(), at_ms: e.at_ms });
    // Verdicts: the newest of each reviewer kind. Stale when the checks of a newer head came after it.
    let mut verdicts: Vec<ReviewsVerdict> = Vec::new();
    for by in [PrFlowBy::Bot, PrFlowBy::Codex, PrFlowBy::Person] {
        let mine: Vec<&ReviewFlowEvent> = ev.review.iter().copied().filter(|e| e.by == by).collect();
        if let Some(e) = newest(&mine, |e| e.at_ms) {
            let moved = match (&e.head, &checks) {
                (Some(h), Some(c)) => c.head.as_ref().is_some_and(|ch| ch != h) && c.at_ms >= e.at_ms,
                _ => false,
            };
            verdicts.push(ReviewsVerdict { by, state: e.state.clone(), head: e.head.clone(), at_ms: e.at_ms, stale: moved });
        }
    }
    let merge_event = ev.merge.iter().max_by_key(|e| e.at_ms).map(|e| e.state.clone());
    let merge = merge_event.or_else(|| card.and_then(|c| c.merged_at_ms).map(|_| "merged".to_string())).unwrap_or_else(|| "open".into());
    let pairs: Vec<(&str, bool)> = verdicts.iter().map(|v| (v.state.as_str(), v.stale)).collect();
    let task_state = card.map(|c| c.task.state.as_str());
    let (waits, why) = waits_on(&WaitsInput { merge: &merge, checks: checks.as_ref().map(|c| c.state.as_str()), reviews: &pairs, task_state });
    let latest_event = ev
        .review
        .iter()
        .map(|e| e.at_ms)
        .chain(ev.checks.iter().map(|e| e.at_ms))
        .chain(ev.merge.iter().map(|e| e.at_ms))
        .max()
        .map_or(0, |ms| ms as i64);
    let agent = card.map(|c| c.agent.clone()).filter(|a| !a.is_empty()).or_else(|| orphan_agent.map(str::to_string));
    ReviewRow {
        repo: card.map(|c| c.repo.clone()),
        repo_id: key.map(|k| k.0),
        number: card.map(|c| c.number).or(key.map(|k| k.1)).unwrap_or(0),
        url: card.map(|c| format!("https://github.com/{}/pull/{}", c.repo, c.number)),
        task: card.map(|c| c.task.clone()),
        agent,
        branch: card.map(|c| c.branch.clone()).unwrap_or_default(),
        reviews: verdicts,
        checks,
        merge,
        merged_at_ms: card.and_then(|c| c.merged_at_ms),
        released_in: card.map(|c| c.released_in.clone()).unwrap_or_default(),
        waits_on: waits,
        why: why.to_string(),
        updated_at_ms: latest_event.max(card.map_or(0, |c| c.updated_at_ms)),
    }
}

/// The list: a row for each card that names a pull request, and one for each pull request an event named that
/// no card holds, newest change first. `cards` is whatever the caller may see.
pub fn build(cards: &[CardIn], flow: &PrFlowSnapshot, now_ms: i64) -> ReviewsList {
    let bound = bind(cards, flow);
    let mut by_card: HashMap<usize, EventKey> = HashMap::new();
    for (key, &i) in &bound {
        // A card holds one PR; if two keys claim it, the older binding (lower repo id) keeps it.
        by_card.entry(i).and_modify(|k| *k = (*k).min(*key)).or_insert(*key);
    }
    let mut rows: Vec<ReviewRow> = Vec::new();
    for (i, card) in cards.iter().enumerate() {
        let key = by_card.get(&i).copied();
        let ev = key.map(|k| events_of(flow, k)).unwrap_or(Events { review: vec![], checks: vec![], merge: vec![] });
        rows.push(row_of(&ev, key, Some(card), None));
    }
    // Pull requests the events named and no card holds.
    let used: BTreeSet<EventKey> = by_card.values().copied().collect();
    let mut orphans: HashMap<EventKey, String> = HashMap::new();
    let all = flow
        .review
        .iter()
        .map(|e| (e.pr, e.agent.as_str()))
        .chain(flow.checks.iter().map(|e| (e.pr, e.agent.as_str())))
        .chain(flow.merge.iter().map(|e| (e.pr, e.agent.as_str())));
    for (pr, agent) in all {
        let key = (pr.repo_id, pr.number);
        if !used.contains(&key) {
            orphans.entry(key).or_insert_with(|| agent.to_string());
        }
    }
    for (key, agent) in orphans {
        rows.push(row_of(&events_of(flow, key), Some(key), None, Some(&agent)));
    }
    rows.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then(a.number.cmp(&b.number)));
    ReviewsList { rows, as_of_ms: now_ms, last_notice_at_ms: flow.bot.last_notice_at_ms }
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
