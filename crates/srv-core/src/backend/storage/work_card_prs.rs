// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A card's pull requests: every one of them, in every repository, each with its state
//! (the Kanban and GitHub spec, §3). The card's own `pr_url` stays as the newest
//! open one; this list is what the board derives its "merged" and "open work" from.
//!
//! Kept in `db_work_card_prs`, beside the board's other tables, so an older build on the same computer never
//! sees it. A row is written by more than one source (the holder, a hand-in, a person, a push event, a
//! signed-in read), and every writer is **idempotent**: a report older than the row is ignored, and a merge
//! is never undone, so two sources agree whichever arrives first (§6).
//!
//! A pull request a person or the holder took off the card is kept, marked `unlinked`, so no later report
//! puts it back; only an explicit link does.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::store::Store;
use crate::backend::reviews::{waits_on, WaitsInput};
use super::work_board::BoardTask;
use super::work_queue::work_state;
use super::StoreError;

/// The table, created with the board's (`work_board::SCHEMA`).
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_work_card_prs (
        work_id    TEXT NOT NULL,
        repo       TEXT NOT NULL,
        number     INTEGER NOT NULL,
        state      TEXT NOT NULL DEFAULT 'open',
        head       TEXT NOT NULL DEFAULT '',
        reviews    TEXT NOT NULL DEFAULT '{}',
        checks     TEXT NOT NULL DEFAULT 'none',
        opened_at  INTEGER,
        merged_at  INTEGER,
        updated_at INTEGER NOT NULL DEFAULT 0,
        source     TEXT NOT NULL DEFAULT 'holder',
        unlinked   INTEGER NOT NULL DEFAULT 0,
        linked_at  INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (work_id, repo, number)
    );
    CREATE INDEX IF NOT EXISTS idx_work_card_prs_pr ON db_work_card_prs (repo, number);";

/// What the review, checks and merge notices add to a row (SPEC_FOLD_REVIEWS_INTO_THE_KANBAN_2026_11_22.md §3): the
/// repository's numeric id once learned, the commit the checks were for and when, and the merge-queue state and when.
/// Added to a table an earlier build created, so a default needs no schema version bump.
pub(crate) const COLUMNS: [&str; 5] = [
    "ALTER TABLE db_work_card_prs ADD COLUMN repo_id INTEGER",
    "ALTER TABLE db_work_card_prs ADD COLUMN checks_head TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE db_work_card_prs ADD COLUMN checks_at INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE db_work_card_prs ADD COLUMN flow_merge TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE db_work_card_prs ADD COLUMN flow_at INTEGER NOT NULL DEFAULT 0",
];

/// The columns a row is read with.
const PR_COLS: &str =
    "work_id, repo, number, state, head, reviews, checks, opened_at, merged_at, updated_at, source, repo_id, checks_head, checks_at, flow_merge, flow_at";

/// Task ids per lookup query, well under SQLite's bound-parameter limit.
const IDS_PER_QUERY: usize = 500;

/// Where a card's link to a pull request came from. A link made by the holder, a hand-in or a person is the
/// task's own; one made by a tag in a push event alone (`Event`) or by a read is weaker, and never closes a
/// task (§10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrSource {
    Event,
    Read,
    HandIn,
    Holder,
    Person,
}

impl PrSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PrSource::Event => "event",
            PrSource::Read => "read",
            PrSource::HandIn => "hand_in",
            PrSource::Holder => "holder",
            PrSource::Person => "person",
        }
    }

    fn parse(s: &str) -> PrSource {
        match s {
            "event" => PrSource::Event,
            "read" => PrSource::Read,
            "hand_in" => PrSource::HandIn,
            "person" => PrSource::Person,
            _ => PrSource::Holder,
        }
    }

    /// Stronger beats weaker when two sources name the same link: a person over the holder over a hand-in
    /// over a read over a tag.
    fn strength(self) -> u8 {
        match self {
            PrSource::Event => 0,
            PrSource::Read => 1,
            PrSource::HandIn => 2,
            PrSource::Holder => 3,
            PrSource::Person => 4,
        }
    }

    /// Whether a task may be closed on the strength of a link from here.
    pub fn may_close(self) -> bool {
        !matches!(self, PrSource::Event | PrSource::Read)
    }
}

/// What GitHub says a pull request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardPrState {
    Open,
    Merged,
    /// Closed without merging.
    Closed,
}

impl CardPrState {
    pub fn as_str(self) -> &'static str {
        match self {
            CardPrState::Open => "open",
            CardPrState::Merged => "merged",
            CardPrState::Closed => "closed",
        }
    }
}

/// One reviewer kind's latest verdict on a pull request, as a notice gave it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrVerdict {
    /// `bot`, `codex` or `person`.
    pub by: String,
    /// `approved`, `changes_requested`, `commented`, or another well-formed name.
    pub state: String,
    /// The commit it was given on, when the notice named one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub head: String,
    /// When srv accepted the notice, in milliseconds since 1970.
    pub at: i64,
    /// A newer commit was named after it: it was given on an older head. Derived on read, never stored.
    #[serde(default)]
    pub stale: bool,
}

/// One pull request of a card.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkCardPr {
    /// `owner/name`, lower case.
    pub repo: String,
    pub number: u64,
    /// Built from `repo` and `number`, never taken from a report's text.
    pub url: String,
    /// `open`, `merged` or `closed` (closed without merging).
    pub state: String,
    /// Its head commit when last reported (an identifier).
    pub head: String,
    /// `none`, `approved` or `changes_requested`, derived from the latest review of each kind on the head.
    pub review: String,
    /// `none`, `pending`, `passing` or `failing` on the head.
    pub checks: String,
    pub opened_at: Option<i64>,
    pub merged_at: Option<i64>,
    /// GitHub's own time for the last report applied, used to ignore an older one.
    pub updated_at: i64,
    /// One of [`PrSource`]: where the link came from.
    pub source: String,
    /// The latest verdict of each reviewer kind (the review, checks and merge notices srv was told of).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verdicts: Vec<PrVerdict>,
    /// Who has to act next: `person`, `author`, `reviewer`, `ci`, `queue`, `none` or `unknown`; empty before it is derived.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub waits_on: String,
    /// A short code for why (`changes_requested`, `checks_failing`, `ready_to_merge`, ...).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// GitHub's numeric id of the repository, once a notice settled it. Not sent to the app.
    #[serde(skip)]
    pub repo_id: Option<i64>,
    /// The commit the checks were for, and when they were reported. Not sent to the app.
    #[serde(skip)]
    pub checks_head: String,
    #[serde(skip)]
    pub checks_at: i64,
    /// The merge or queue state a notice gave, and when. Not sent to the app.
    #[serde(skip)]
    pub flow_merge: String,
    #[serde(skip)]
    pub flow_at: i64,
    /// `open`, `queued`, `merged` or `closed`, once derived; the merge half of `waits_on`'s input. Not sent to the app.
    #[serde(skip)]
    pub merge: String,
}

impl WorkCardPr {
    /// Whether a task may be closed on the strength of this link.
    pub fn may_close(&self) -> bool {
        PrSource::parse(&self.source).may_close()
    }
}

/// What the card's list of pull requests adds up to, derived and never stored (§3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrRollup {
    pub total: usize,
    pub open: usize,
    pub merged: usize,
    pub closed_unmerged: usize,
    /// `none` without a pull request; `open` while any is open (open work); `merged` when at least one merged
    /// and every one not closed unmerged has; `closed` when each was closed unmerged.
    pub state: String,
    /// The card's task is finished and a pull request is open again (a follow-up after Done).
    pub follow_up_open: bool,
    /// The task is held, and every pull request of it has merged: its holder is waiting to complete it.
    pub all_merged_waiting: bool,
    /// What the pull request that blocks the card waits on (see [`WorkCardPr::waits_on`]); empty when no open pull
    /// request has been placed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub waits_on: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// The pull request that blocks the card, as `owner/repo#12`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blocking: String,
    /// How many other open pull requests the card has besides it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub waits_more: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The rollup of a card's (linked) pull requests for a task in `task_state`.
pub fn rollup(prs: &[WorkCardPr], task_state: &str) -> PrRollup {
    let count = |s: &str| prs.iter().filter(|p| p.state == s).count();
    let (open, merged, closed_unmerged) = (count("open"), count("merged"), count("closed"));
    let state = if prs.is_empty() {
        "none"
    } else if open > 0 {
        "open"
    } else if merged > 0 {
        "merged"
    } else {
        "closed"
    };
    let held = matches!(task_state, work_state::CLAIMED | work_state::BLOCKED);
    let blocking = blocking_pr(prs);
    PrRollup {
        total: prs.len(),
        open,
        merged,
        closed_unmerged,
        state: state.into(),
        follow_up_open: task_state == work_state::DONE && open > 0 && merged > 0,
        all_merged_waiting: held && merged > 0 && open == 0,
        waits_on: blocking.map(|p| p.waits_on.clone()).unwrap_or_default(),
        why: blocking.map(|p| p.why.clone()).unwrap_or_default(),
        blocking: blocking.map(|p| format!("{}#{}", p.repo, p.number)).unwrap_or_default(),
        waits_more: if blocking.is_some() { open.saturating_sub(1) } else { 0 },
    }
}

/// How pressing a derived `waits_on` is, lowest first (a person, the author, a reviewer, CI, the queue, unknown, nothing).
fn urgency(waits_on: &str) -> u8 {
    match waits_on {
        "person" => 0,
        "author" => 1,
        "reviewer" => 2,
        "ci" => 3,
        "queue" => 4,
        "unknown" => 5,
        _ => 6,
    }
}

/// The open pull request that blocks the card: the one whose wait is the most pressing, the most recently changed
/// when two tie. None when no open pull request has been placed (its wait is not derived yet).
fn blocking_pr(prs: &[WorkCardPr]) -> Option<&WorkCardPr> {
    prs.iter()
        .filter(|p| p.state == "open" && !p.waits_on.is_empty())
        .min_by_key(|p| (urgency(&p.waits_on), std::cmp::Reverse(p.updated_at.max(p.checks_at).max(p.flow_at))))
}

/// The verdicts a `reviews` column holds, each reviewer kind's latest. An older build wrote a bare state per kind.
fn verdicts_of(reviews: &str) -> BTreeMap<String, PrVerdict> {
    let Ok(map) = serde_json::from_str::<BTreeMap<String, Value>>(reviews) else { return BTreeMap::new() };
    map.into_iter()
        .filter_map(|(by, v)| {
            let verdict = match &v {
                Value::String(state) => PrVerdict { by: by.clone(), state: state.clone(), ..Default::default() },
                Value::Object(o) => PrVerdict {
                    by: by.clone(),
                    state: o.get("s")?.as_str()?.to_string(),
                    head: o.get("h").and_then(Value::as_str).unwrap_or_default().to_string(),
                    at: o.get("at").and_then(Value::as_i64).unwrap_or(0),
                    stale: false,
                },
                _ => return None,
            };
            Some((by, verdict))
        })
        .collect()
}

/// The `reviews` column for `verdicts`.
fn verdicts_json(verdicts: &BTreeMap<String, PrVerdict>) -> String {
    let map: serde_json::Map<String, Value> = verdicts
        .iter()
        .map(|(by, v)| (by.clone(), serde_json::json!({ "s": v.state, "h": v.head, "at": v.at })))
        .collect();
    Value::Object(map).to_string()
}

/// Whether two commit ids name one commit (one may be an abbreviation of the other).
fn same_head(a: &str, b: &str) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Work out, from what the notices said, whether each verdict is stale, which checks are current, and what the pull
/// request waits on (`reviews::waits_on`, the one table). A verdict or checks result given on a commit older than the
/// newest one a later notice named is stale or not shown: it is of a head that moved.
fn derive_flow(pr: &mut WorkCardPr, task_state: &str) {
    // The pull request's own head, when a report gave one, is the current commit: a notice that arrives late for an
    // older commit does not become the newest. With none known, the newest commit any notice named is.
    let newest: Option<(String, i64)> = if pr.head.is_empty() {
        pr.verdicts
            .iter()
            .filter(|v| !v.head.is_empty())
            .map(|v| (v.head.clone(), v.at))
            .chain((!pr.checks_head.is_empty()).then(|| (pr.checks_head.clone(), pr.checks_at)))
            .max_by_key(|(_, at)| *at)
    } else {
        Some((pr.head.clone(), i64::MAX))
    };
    for v in &mut pr.verdicts {
        v.stale = match (&newest, v.head.is_empty()) {
            (Some((head, at)), false) => !same_head(head, &v.head) && *at >= v.at,
            _ => false,
        };
    }
    if !pr.checks_head.is_empty() && newest.as_ref().is_some_and(|(head, _)| !same_head(head, &pr.checks_head)) {
        pr.checks = "none".into();
    }
    // Approved or changes requested by the verdicts that are still of the current head.
    let fresh: Vec<&PrVerdict> = pr.verdicts.iter().filter(|v| !v.stale).collect();
    pr.review = if fresh.iter().any(|v| v.state == "changes_requested") {
        "changes_requested"
    } else if !fresh.is_empty() && fresh.iter().all(|v| v.state == "approved") {
        "approved"
    } else {
        "none"
    }
    .into();
    let merge = match (pr.state.as_str(), pr.flow_merge.as_str()) {
        ("merged" | "closed", _) => pr.state.as_str(),
        (_, m @ ("merged" | "closed" | "queued")) => m,
        _ => "open",
    };
    let pairs: Vec<(&str, bool)> = pr.verdicts.iter().map(|v| (v.state.as_str(), v.stale)).collect();
    let checks = (pr.checks != "none").then_some(pr.checks.as_str());
    let (waits, why) = waits_on(&WaitsInput { merge, checks, reviews: &pairs, task_state: Some(task_state) });
    pr.merge = merge.into();
    pr.waits_on = waits.as_str().into();
    pr.why = why.into();
}

/// `owner/repo`, as the card stores a repository from its `origin` remote; not a local folder (`local:…`).
pub fn github_repo(repo: &str) -> Option<String> {
    let repo = repo.trim();
    let (owner, name) = repo.split_once('/')?;
    let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (ok(owner) && ok(name)).then(|| repo.to_ascii_lowercase())
}

/// The pull request a card's link names (repository and number): a GitHub PR address, `owner/repo#12`, or
/// `#12` / `12` in the card's repository.
pub fn parse_pr_ref(link: &str, card_repo: &str) -> Option<(String, u64)> {
    let link = link.trim();
    if let Some((_, path)) = link.split_once("github.com/") {
        let parts: Vec<&str> = path.split(['/', '?', '#']).collect();
        if let [owner, repo, "pull" | "pulls", n, ..] = parts.as_slice() {
            return Some((github_repo(&format!("{owner}/{repo}"))?, n.parse().ok()?));
        }
        return None;
    }
    let (repo, n) = match link.split_once('#') {
        Some((r, n)) if !r.is_empty() => (github_repo(r)?, n),
        Some((_, n)) => (github_repo(card_repo)?, n),
        None => (github_repo(card_repo)?, link),
    };
    Some((repo, n.parse().ok().filter(|n| *n > 0)?))
}

/// The address of a pull request.
pub fn pr_url(repo: &str, number: u64) -> String {
    format!("https://github.com/{repo}/pull/{number}")
}

/// What a source learned about one pull request. `updated_at` is GitHub's own time for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrReport {
    pub repo: String,
    pub number: u64,
    pub state: CardPrState,
    pub head: Option<String>,
    pub opened_at: Option<i64>,
    pub merged_at: Option<i64>,
    pub updated_at: i64,
}

/// What a write did to the card's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Applied {
    /// A row was added, changed or put back.
    pub changed: bool,
    /// The pull request went from not-merged to merged with this write.
    pub newly_merged: bool,
}

fn row_to_pr(repo: String, number: i64, r: &rusqlite::Row) -> rusqlite::Result<WorkCardPr> {
    let reviews: String = r.get("reviews")?;
    Ok(WorkCardPr {
        url: pr_url(&repo, number as u64),
        repo,
        number: number as u64,
        state: r.get("state")?,
        head: r.get("head")?,
        review: review_of(&reviews),
        checks: r.get("checks")?,
        opened_at: r.get("opened_at")?,
        merged_at: r.get("merged_at")?,
        updated_at: r.get("updated_at")?,
        source: r.get("source")?,
        verdicts: verdicts_of(&reviews).into_values().collect(),
        repo_id: r.get("repo_id")?,
        checks_head: r.get("checks_head")?,
        checks_at: r.get("checks_at")?,
        flow_merge: r.get("flow_merge")?,
        flow_at: r.get("flow_at")?,
        ..Default::default()
    })
}

/// The pull request's review, from the latest review of each kind on its head (a JSON object, kind to its verdict):
/// approved when every kind that reviewed approved, changes requested when any asked for changes, otherwise none
/// (§3). [`derive_flow`] redoes it over the verdicts that are not stale.
fn review_of(reviews: &str) -> String {
    let map = verdicts_of(reviews);
    if map.values().any(|v| v.state == "changes_requested") {
        "changes_requested".into()
    } else if !map.is_empty() && map.values().all(|v| v.state == "approved") {
        "approved".into()
    } else {
        "none".into()
    }
}

/// Count it as a change of the task, so a finished one stays on the board its week and boards elsewhere
/// see it move.
fn touch(tx: &Transaction, work_id: &str, now_ms: i64) -> Result<(), StoreError> {
    tx.execute("UPDATE db_work_queue SET updated_at = MAX(updated_at, ?2) WHERE id = ?1", params![work_id, now_ms])?;
    tx.execute("UPDATE db_work_cards SET updated_at = MAX(updated_at, ?2) WHERE work_id = ?1", params![work_id, now_ms])?;
    Ok(())
}

/// Link `work_id` to a pull request, from `source`. A new row starts `open`, with nothing known. A link
/// already there stays, a stronger source takes its place, and a pull request that was taken off the card
/// comes back only for a holder or a person (`explicit`). Returns whether the list changed.
pub(crate) fn link_in(tx: &Transaction, work_id: &str, repo: &str, number: u64, source: PrSource, now_ms: i64) -> Result<bool, StoreError> {
    let existing: Option<(String, bool)> = tx
        .query_row(
            "SELECT source, unlinked FROM db_work_card_prs WHERE work_id = ?1 AND repo = ?2 AND number = ?3",
            params![work_id, repo, number as i64],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
        )
        .optional()?;
    let changed = match existing {
        None => {
            tx.execute(
                "INSERT INTO db_work_card_prs (work_id, repo, number, source, linked_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![work_id, repo, number as i64, source.as_str(), now_ms],
            )?;
            true
        }
        Some((_, true)) if matches!(source, PrSource::Holder | PrSource::Person) => {
            tx.execute(
                "UPDATE db_work_card_prs SET unlinked = 0, source = ?4, linked_at = ?5 WHERE work_id = ?1 AND repo = ?2 AND number = ?3",
                params![work_id, repo, number as i64, source.as_str(), now_ms],
            )?;
            true
        }
        // Taken off the card: nothing but an explicit link puts it back.
        Some((_, true)) => false,
        Some((was, false)) if PrSource::parse(&was).strength() < source.strength() => {
            tx.execute(
                "UPDATE db_work_card_prs SET source = ?4 WHERE work_id = ?1 AND repo = ?2 AND number = ?3",
                params![work_id, repo, number as i64, source.as_str()],
            )?;
            true
        }
        Some(_) => false,
    };
    if changed {
        touch(tx, work_id, now_ms)?;
    }
    Ok(changed)
}

/// Apply what a source learned. Older than the row, or the same moment, is ignored; a merge is never undone;
/// a pull request taken off the card stays off.
fn apply_in(tx: &Transaction, work_id: &str, r: &PrReport, source: PrSource, now_ms: i64) -> Result<Applied, StoreError> {
    link_in(tx, work_id, &r.repo, r.number, source, now_ms)?;
    let (state, unlinked, updated_at, merged_at): (String, i64, i64, Option<i64>) = tx.query_row(
        "SELECT state, unlinked, updated_at, merged_at FROM db_work_card_prs WHERE work_id = ?1 AND repo = ?2 AND number = ?3",
        params![work_id, r.repo, r.number as i64],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    if unlinked != 0 {
        return Ok(Applied::default());
    }
    let was_merged = state == "merged";
    // Not newer than what the row holds: nothing to learn (a row nobody has read yet holds 0).
    if r.updated_at <= updated_at || (was_merged && r.state != CardPrState::Merged) {
        return Ok(Applied::default());
    }
    let merged_at = match r.state {
        CardPrState::Merged => r.merged_at.or(merged_at).or(Some(now_ms)),
        _ => merged_at,
    };
    tx.execute(
        "UPDATE db_work_card_prs
            SET state = ?4, head = COALESCE(?5, head), opened_at = COALESCE(opened_at, ?6), merged_at = ?7, updated_at = ?8
          WHERE work_id = ?1 AND repo = ?2 AND number = ?3",
        params![work_id, r.repo, r.number as i64, r.state.as_str(), r.head, r.opened_at, merged_at, r.updated_at],
    )?;
    touch(tx, work_id, now_ms)?;
    Ok(Applied { changed: true, newly_merged: !was_merged && r.state == CardPrState::Merged })
}

/// Fill each task's `card.prs` and `card.pr_rollup`.
pub(crate) fn attach_card_prs(conn: &Connection, tasks: &mut [BoardTask]) -> Result<(), StoreError> {
    let ids: Vec<String> = tasks.iter().map(|t| t.item.id.clone()).collect();
    let mut found: HashMap<String, Vec<WorkCardPr>> = HashMap::new();
    for chunk in ids.chunks(IDS_PER_QUERY) {
        let marks = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT {PR_COLS}
               FROM db_work_card_prs WHERE unlinked = 0 AND work_id IN ({marks})
              ORDER BY COALESCE(opened_at, linked_at) DESC, number DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |r| {
            let (id, repo, number): (String, String, i64) = (r.get("work_id")?, r.get("repo")?, r.get("number")?);
            Ok((id, row_to_pr(repo, number, r)?))
        })?;
        for row in rows {
            let (id, pr) = row?;
            found.entry(id).or_default().push(pr);
        }
    }
    for t in tasks {
        let mut prs = found.remove(&t.item.id).unwrap_or_default();
        prs.iter_mut().for_each(|p| derive_flow(p, &t.item.state));
        t.card.pr_rollup = rollup(&prs, &t.item.state);
        t.card.prs = prs;
    }
    Ok(())
}

/// What a review, checks or merge notice says about a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowFact {
    /// A reviewer kind's verdict, on a commit when the notice named one.
    Review { by: String, state: String, head: Option<String> },
    /// The checks of a commit, when the notice named one.
    Checks { state: String, head: Option<String> },
    /// The merge or queue state (`merged`, `closed`, `queued`, ...).
    Merge { state: String },
}

/// One accepted notice, for the pull request `repo_id#number` of the local agent `agent` (the notices name a
/// repository only by its numeric id, and go to the pull request's own agent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowNote {
    pub agent: String,
    pub repo_id: u64,
    pub number: u64,
    /// When srv accepted the notice, in milliseconds since 1970.
    pub at_ms: i64,
    pub fact: FlowFact,
}

impl Store {
    /// Put what a notice says on the card rows of the pull request it names. The card is found by the holding agent
    /// and the number; the repository's id is learned only from one unambiguous match (exactly one repository among
    /// the agent's linked pull requests with that number, and the id not already another repository's), and from then
    /// on every card row of that repository and number takes the notice. Two cards of one agent that hold the same
    /// number in different repositories stay unplaced until a notice for one of them settles it: never a guess.
    /// A notice older than what a row holds of its kind is ignored. Returns how many rows changed.
    pub fn work_card_pr_flow_apply(&self, note: &FlowNote, now_ms: i64) -> Result<usize, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let id = note.repo_id as i64;
        let candidates: Vec<(String, Option<i64>)> = {
            let mut stmt = tx.prepare(
                "SELECT r.repo, r.repo_id FROM db_work_card_prs r JOIN db_work_queue q ON q.id = r.work_id
                  WHERE r.number = ?1 AND r.unlinked = 0
                    AND ((q.claimed_by <> '' AND q.claimed_by = ?2 COLLATE NOCASE)
                      OR (q.claimed_by = '' AND q.target_agent = ?2 COLLATE NOCASE))",
            )?;
            let rows = stmt.query_map(params![note.number as i64, note.agent], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let repo = match candidates.iter().find(|(_, known)| *known == Some(id)) {
            Some((repo, _)) => repo.clone(),
            None => {
                let repos: BTreeSet<&str> = candidates.iter().map(|(r, _)| r.as_str()).collect();
                let mut it = repos.into_iter();
                let (Some(only), None) = (it.next(), it.next()) else { return Ok(0) };
                // One id is one repository, and a repository has one id.
                let clash: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM db_work_card_prs
                      WHERE (repo_id = ?1 AND repo <> ?2) OR (repo = ?2 AND repo_id IS NOT NULL AND repo_id <> ?1)",
                    params![id, only],
                    |r| r.get(0),
                )?;
                if clash > 0 {
                    return Ok(0);
                }
                tx.execute("UPDATE db_work_card_prs SET repo_id = ?1 WHERE repo = ?2", params![id, only])?;
                only.to_string()
            }
        };
        let rows: Vec<(String, String, i64, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT work_id, reviews, checks_at, flow_at FROM db_work_card_prs WHERE repo = ?1 AND number = ?2 AND unlinked = 0",
            )?;
            let found = stmt.query_map(params![repo, note.number as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            found.collect::<Result<Vec<_>, _>>()?
        };
        let mut changed = 0;
        for (work_id, reviews, checks_at, flow_at) in rows {
            let wrote = match &note.fact {
                FlowFact::Review { by, state, head } => {
                    let mut verdicts = verdicts_of(&reviews);
                    if verdicts.get(by).is_some_and(|v| v.at > note.at_ms) {
                        0
                    } else {
                        let head = head.clone().unwrap_or_default();
                        verdicts.insert(by.clone(), PrVerdict { by: by.clone(), state: state.clone(), head, at: note.at_ms, stale: false });
                        tx.execute(
                            "UPDATE db_work_card_prs SET reviews = ?4 WHERE work_id = ?1 AND repo = ?2 AND number = ?3",
                            params![work_id, repo, note.number as i64, verdicts_json(&verdicts)],
                        )?
                    }
                }
                FlowFact::Checks { state, head } => {
                    if checks_at > note.at_ms {
                        0
                    } else {
                        let state = if matches!(state.as_str(), "passing" | "failing") { state.as_str() } else { "pending" };
                        tx.execute(
                            "UPDATE db_work_card_prs SET checks = ?4, checks_head = ?5, checks_at = ?6 WHERE work_id = ?1 AND repo = ?2 AND number = ?3",
                            params![work_id, repo, note.number as i64, state, head.as_deref().unwrap_or(""), note.at_ms],
                        )?
                    }
                }
                FlowFact::Merge { state } => {
                    if flow_at > note.at_ms {
                        0
                    } else {
                        tx.execute(
                            "UPDATE db_work_card_prs SET flow_merge = ?4, flow_at = ?5 WHERE work_id = ?1 AND repo = ?2 AND number = ?3",
                            params![work_id, repo, note.number as i64, state, note.at_ms],
                        )?
                    }
                }
            };
            if wrote > 0 {
                touch(&tx, &work_id, now_ms)?;
                changed += 1;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// A task's linked pull requests, newest first.
    pub fn work_card_prs(&self, work_id: &str) -> Result<Vec<WorkCardPr>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {PR_COLS}
               FROM db_work_card_prs WHERE unlinked = 0 AND work_id = ?1
              ORDER BY COALESCE(opened_at, linked_at) DESC, number DESC"
        ))?;
        let rows = stmt.query_map(params![work_id], |r| {
            let (repo, number): (String, i64) = (r.get("repo")?, r.get("number")?);
            row_to_pr(repo, number, r)
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Link a task to a pull request from `source` (a holder, a hand-in, a person, a tag). Returns whether the
    /// list changed.
    pub fn work_card_pr_link(&self, work_id: &str, repo: &str, number: u64, source: PrSource, now_ms: i64) -> Result<bool, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let changed = link_in(&tx, work_id, repo, number, source, now_ms)?;
        tx.commit()?;
        Ok(changed)
    }

    /// Take a pull request off a task's card. It stays recorded, so no later report puts it back. Returns
    /// whether it was on the card.
    pub fn work_card_pr_unlink(&self, work_id: &str, repo: &str, number: u64, now_ms: i64) -> Result<bool, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE db_work_card_prs SET unlinked = 1 WHERE work_id = ?1 AND repo = ?2 AND number = ?3 AND unlinked = 0",
            params![work_id, repo, number as i64],
        )?;
        if n > 0 {
            // The card's own address follows the list: when it named this pull request, it moves to the newest
            // one left (or empties), and what was learned about the old one goes with it.
            let (card_repo, card_url): (String, String) = tx
                .query_row("SELECT repo, pr_url FROM db_work_cards WHERE work_id = ?1", params![work_id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?
                .unwrap_or_default();
            if parse_pr_ref(&card_url, &card_repo) == Some((repo.to_string(), number)) {
                let next: Option<(String, i64)> = tx
                    .query_row(
                        "SELECT repo, number FROM db_work_card_prs WHERE work_id = ?1 AND unlinked = 0
                          ORDER BY linked_at DESC, rowid DESC LIMIT 1",
                        params![work_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                let url = next.map(|(r, n)| pr_url(&r, n as u64)).unwrap_or_default();
                tx.execute("UPDATE db_work_cards SET pr_url = ?2 WHERE work_id = ?1", params![work_id, url])?;
                tx.execute("DELETE FROM db_work_prs WHERE work_id = ?1", params![work_id])?;
            }
            touch(&tx, work_id, now_ms)?;
        }
        tx.commit()?;
        Ok(n > 0)
    }

    /// Apply what a source learned about one of a task's pull requests: link it (a new row, from `source`)
    /// and record its state, unless an equal or newer report is already there.
    pub fn work_card_pr_apply(&self, work_id: &str, report: &PrReport, source: PrSource, now_ms: i64) -> Result<Applied, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let applied = apply_in(&tx, work_id, report, source, now_ms)?;
        tx.commit()?;
        Ok(applied)
    }

    /// Make the list carry what `pr_url` says for each card that has an address and no list yet (cards from
    /// before the list existed), with its merge as `db_work_prs` recorded it. Safe to run on every start.
    /// Returns how many cards it gave a row.
    pub fn work_card_prs_backfill(&self, now_ms: i64) -> Result<usize, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let cards: Vec<(String, String, String, Option<i64>)> = {
            let mut stmt = tx.prepare(
                "SELECT c.work_id, c.repo, c.pr_url, p.merged_at
                   FROM db_work_cards c LEFT JOIN db_work_prs p ON p.work_id = c.work_id
                  WHERE c.pr_url <> ''
                    AND NOT EXISTS (SELECT 1 FROM db_work_card_prs x WHERE x.work_id = c.work_id)",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut made = 0;
        for (work_id, card_repo, url, merged_at) in cards {
            let Some((repo, number)) = parse_pr_ref(&url, &card_repo) else { continue };
            tx.execute(
                "INSERT OR IGNORE INTO db_work_card_prs (work_id, repo, number, state, merged_at, updated_at, source, linked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'holder', ?7)",
                params![
                    work_id,
                    repo,
                    number as i64,
                    if merged_at.is_some() { "merged" } else { "open" },
                    merged_at,
                    merged_at.unwrap_or(0),
                    now_ms
                ],
            )?;
            made += 1;
        }
        tx.commit()?;
        Ok(made)
    }

    /// Drop the lists of tasks no longer in the queue.
    pub fn work_card_prs_prune(&self) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute("DELETE FROM db_work_card_prs WHERE work_id NOT IN (SELECT id FROM db_work_queue)", [])?)
    }
}

#[cfg(test)]
#[path = "work_card_prs_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "work_card_pr_flow_tests.rs"]
mod flow_tests;
