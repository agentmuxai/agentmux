// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Kanban and GitHub (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md §6.9, D5):
//!
//! - **Finding the PR.** When an agent holding one task opens a PR, the card
//!   gets the PR's address once GitHub lists it, so the card links to it
//!   without the agent naming it. A result that names one PR ("PR #12") does
//!   the same at hand-in, when that PR is from the branch the agent is on: a
//!   result that only mentions someone else's PR links nothing, so that PR's
//!   merge can't close the task.
//! - **Closing on merge.** A task in Review whose PR has merged is accepted:
//!   the merge was the review. Off with the setting `kanban:closeonmerge`.
//! - **Merged, then Released** (SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md
//!   §4): a task's PR that merged is recorded with when (`work_prs.rs`), so
//!   the board can show the task in Merged even while its agent still holds
//!   it; and once a release of the repository was made from a commit at or
//!   after that merge, the task is recorded as in that release (Released).
//!   `main` is linear (a merge queue), so a release made after a merge
//!   contains it.
//!
//! srv holds no GitHub credential, so both read GitHub's public API without
//! one: they work for public repositories, and a private one stays as it was
//! (the agent links its PR with `Tasks`, the operator accepts by hand). The
//! unauthenticated API allows 60 calls an hour per address, shared with
//! everything else on the computer, so this spends at most [`HOURLY_BUDGET`]
//! and stops until GitHub's reset when told it is over.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use serde_json::Value;

use super::{publish_changed, CONTEXT};
use crate::backend::storage::store::Store;
use crate::backend::storage::work_board::{BoardTask, CardUpdate, WorkEvent};
use crate::backend::storage::work_queue::work_state;

/// The setting that stops a merged PR closing its task (default on).
pub const SETTING_CLOSE_ON_MERGE: &str = "kanban:closeonmerge";
const API: &str = "https://api.github.com";
/// How often the loop looks for tasks to check.
const TICK_MS: u64 = 60_000;
/// How often one task's PR is asked about while open.
const CHECK_EVERY_MS: i64 = 10 * 60 * 1000;
/// ... and once it was closed without merging (it may be reopened).
const CLOSED_CHECK_EVERY_MS: i64 = 60 * 60 * 1000;
/// A repository GitHub doesn't show without a login isn't asked about again
/// for this long.
const PRIVATE_FOR_MS: i64 = 6 * 60 * 60 * 1000;
/// The most calls this srv makes in an hour.
pub const HOURLY_BUDGET: usize = 30;
/// When to look for a just-opened PR, in seconds after the command was seen.
const FIND_AFTER_S: [u64; 4] = [20, 60, 180, 600];
/// How often one repository's releases are asked about, while a merged task
/// of it waits to be released.
const RELEASES_EVERY_MS: i64 = 30 * 60 * 1000;

/// A PR on GitHub: `owner/repo` and its number.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrRef {
    pub repo: String,
    pub number: u64,
}

impl PrRef {
    pub fn url(&self) -> String {
        format!("https://github.com/{}/pull/{}", self.repo, self.number)
    }
}

/// `owner/repo`, as the card stores a repository from its `origin` remote;
/// not a local folder (`local:…`).
fn github_repo(repo: &str) -> Option<String> {
    let repo = repo.trim();
    let (owner, name) = repo.split_once('/')?;
    let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (ok(owner) && ok(name)).then(|| repo.to_ascii_lowercase())
}

/// The PR a card's link names: a GitHub PR address, `owner/repo#12`, or
/// `#12` / `12` in the card's repository.
pub fn parse_pr_ref(link: &str, card_repo: &str) -> Option<PrRef> {
    let link = link.trim();
    if let Some((_, path)) = link.split_once("github.com/") {
        let parts: Vec<&str> = path.split(['/', '?', '#']).collect();
        if let [owner, repo, "pull" | "pulls", n, ..] = parts.as_slice() {
            return Some(PrRef { repo: github_repo(&format!("{owner}/{repo}"))?, number: n.parse().ok()? });
        }
        return None;
    }
    let (repo, n) = match link.split_once('#') {
        Some((r, n)) if !r.is_empty() => (github_repo(r)?, n),
        Some((_, n)) => (github_repo(card_repo)?, n),
        None => (github_repo(card_repo)?, link),
    };
    Some(PrRef { repo, number: n.parse().ok().filter(|n| *n > 0)? })
}

/// The one PR a hand-in's text names ("PR #12", a PR address), when it names
/// exactly one; `None` for none or several.
pub fn pr_in_text(text: &str, card_repo: &str) -> Option<PrRef> {
    let mut found: Vec<PrRef> = Vec::new();
    let mut add = |p: Option<PrRef>| {
        if let Some(p) = p {
            if !found.contains(&p) {
                found.push(p);
            }
        }
    };
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ',' | ';' | '<' | '>' | '[' | ']')) {
        let word = word.trim_end_matches(['.', ':', '!']);
        if word.contains("github.com/") {
            add(parse_pr_ref(word, card_repo));
        }
    }
    // "PR #12", "PR 12", "pull request #12".
    let lower = text.to_ascii_lowercase();
    for marker in ["pr #", "pr ", "pull request #", "pr#"] {
        let mut from = 0;
        while let Some(i) = lower[from..].find(marker) {
            let at = from + i;
            from = at + marker.len();
            let before_ok = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric();
            let digits: String = lower[from..].chars().take_while(char::is_ascii_digit).collect();
            if before_ok && !digits.is_empty() {
                add(parse_pr_ref(&digits, card_repo));
            }
        }
    }
    match found.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// What GitHub says about a PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
    /// No such PR in a repository GitHub shows (a mistyped link).
    Missing,
}

/// When GitHub's answer for one PR says it merged (ms).
pub fn merged_at(v: &Value) -> Option<i64> {
    rfc3339_ms(v.get("merged_at")?.as_str()?)
}

fn rfc3339_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

/// The first release (oldest; not a draft) made from a commit at or after
/// `merged_ms`, from GitHub's list of a repository's releases: its tag and
/// when it was published (ms). A release's `created_at` is the date of the
/// commit it was made from.
pub fn release_containing(releases: &Value, merged_ms: i64) -> Option<(String, i64)> {
    releases
        .as_array()?
        .iter()
        .filter(|r| r.get("draft").and_then(Value::as_bool) != Some(true))
        .filter_map(|r| {
            let made = rfc3339_ms(r.get("created_at")?.as_str()?)?;
            let tag = r.get("tag_name")?.as_str()?.to_string();
            let published = r.get("published_at").and_then(Value::as_str).and_then(rfc3339_ms).unwrap_or(made);
            (made >= merged_ms).then_some((made, tag, published))
        })
        .min_by_key(|(made, ..)| *made)
        .map(|(_, tag, published)| (tag, published))
}

/// The state in GitHub's answer for one PR.
pub fn pr_state(v: &Value) -> Option<PrState> {
    if v.get("merged_at").is_some_and(|m| !m.is_null()) || v.get("merged").and_then(Value::as_bool) == Some(true) {
        return Some(PrState::Merged);
    }
    match v.get("state")?.as_str()? {
        "open" => Some(PrState::Open),
        "closed" => Some(PrState::Closed),
        _ => None,
    }
}

/// Why a call came back with nothing.
#[derive(Debug)]
enum Miss {
    /// Not there, or not shown without a login (a private repository).
    NotFound,
    /// Over the limit: nothing more for now.
    Limited,
    Other(String),
}

/// Calls made in the last hour, and when GitHub said to wait until.
struct Budget {
    calls: Mutex<Vec<i64>>,
    wait_until: AtomicI64,
}

static BUDGET: Budget = Budget { calls: Mutex::new(Vec::new()), wait_until: AtomicI64::new(0) };

impl Budget {
    /// Take one call, if the hour's budget and GitHub allow it.
    fn take(&self, now_ms: i64) -> bool {
        if now_ms < self.wait_until.load(Ordering::Relaxed) {
            return false;
        }
        let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        calls.retain(|at| now_ms - at < 3_600_000);
        if calls.len() >= HOURLY_BUDGET {
            return false;
        }
        calls.push(now_ms);
        true
    }
}

async fn get(client: &reqwest::Client, path: &str, now_ms: i64) -> Result<Value, Miss> {
    if !BUDGET.take(now_ms) {
        return Err(Miss::Limited);
    }
    let resp = client
        .get(format!("{API}{path}"))
        .header("User-Agent", concat!("AgentMux/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| Miss::Other(e.to_string()))?;
    let status = resp.status().as_u16();
    if status == 403 || status == 429 {
        let reset_s = resp
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok());
        let until = reset_s.map(|s| s * 1000).unwrap_or(now_ms + 15 * 60_000).max(now_ms + 60_000);
        BUDGET.wait_until.store(until, Ordering::Relaxed);
        return Err(Miss::Limited);
    }
    if status == 404 || status == 401 {
        return Err(Miss::NotFound);
    }
    if !resp.status().is_success() {
        return Err(Miss::Other(format!("GitHub answered {status}")));
    }
    resp.json().await.map_err(|e| Miss::Other(e.to_string()))
}

fn history(identity: &Store, work_id: &str, kind: &str, from: &str, to: &str, body: &str, now_ms: i64) {
    let ev = WorkEvent {
        work_id: work_id.to_string(),
        at: now_ms,
        actor: "GitHub".into(),
        actor_kind: "system".into(),
        kind: kind.to_string(),
        from_state: from.to_string(),
        to_state: to.to_string(),
        body: body.to_string(),
        ..Default::default()
    };
    if let Err(e) = identity.work_event_add(&ev) {
        tracing::debug!(error = %e, "kanban: history line not written");
    }
}

/// Link a task to its PR, unless it has one already. Returns whether it did.
pub fn link(identity: &Store, work_id: &str, pr: &PrRef, now_ms: i64) -> bool {
    let Ok(Some(t)) = identity.work_board_get(work_id) else { return false };
    if !t.card.pr_url.is_empty() {
        return false;
    }
    let url = pr.url();
    if identity.work_card_update(work_id, &CardUpdate { pr_url: Some(url.clone()), ..Default::default() }, now_ms).is_err() {
        return false;
    }
    history(identity, work_id, "linked", &t.item.state, &t.item.state, &url, now_ms);
    true
}

/// A task's PR merged: accept it, if it is still in Review. Returns whether
/// it moved (another srv on this computer may have got there first).
pub fn close_merged(identity: &Store, work_id: &str, pr: &PrRef, now_ms: i64) -> bool {
    match identity.work_accept(work_id, now_ms) {
        Ok(Some(m)) => {
            history(identity, work_id, "accepted", &m.from, &m.to, &format!("PR #{} merged", pr.number), now_ms);
            true
        }
        _ => false,
    }
}

/// Whether closing on merge is on (`kanban:closeonmerge`, default on).
pub fn close_on_merge_enabled(extra: &HashMap<String, Value>) -> bool {
    extra.get(SETTING_CLOSE_ON_MERGE).and_then(Value::as_bool).unwrap_or(true)
}

/// When each task's PR was last asked about, and what came back; when each
/// repository's releases were.
#[derive(Default)]
struct Checked {
    at: HashMap<String, (i64, Option<PrState>)>,
    private_repos: HashMap<String, i64>,
    releases_at: HashMap<String, i64>,
}

/// Whether a task's PR is worth asking about: in Review (its merge closes
/// it), held with a PR in review (Review, then Merged), or finished with a
/// PR whose merge srv hasn't seen yet (its time decides its release).
fn watched(t: &BoardTask) -> bool {
    let merged = t.card.pr.as_ref().is_some_and(|p| p.merged_at.is_some());
    match t.item.state.as_str() {
        work_state::REVIEW => true,
        work_state::CLAIMED | work_state::BLOCKED | work_state::DONE => !merged,
        _ => false,
    }
}

/// The tasks whose PR is due a look, oldest look first.
fn due(tasks: &[BoardTask], checked: &Checked, now_ms: i64) -> Vec<(String, PrRef)> {
    let mut out: Vec<(i64, String, PrRef)> = tasks
        .iter()
        .filter(|t| watched(t))
        .filter_map(|t| Some((t.item.id.clone(), parse_pr_ref(&t.card.pr_url, &t.card.repo)?)))
        .filter(|(_, pr)| checked.private_repos.get(&pr.repo).is_none_or(|at| now_ms - at >= PRIVATE_FOR_MS))
        .filter_map(|(id, pr)| {
            let last = match checked.at.get(&id) {
                None => i64::MIN,
                Some((at, state)) => {
                    let every = match state {
                        Some(PrState::Closed | PrState::Missing) => CLOSED_CHECK_EVERY_MS,
                        _ => CHECK_EVERY_MS,
                    };
                    if now_ms - at < every {
                        return None;
                    }
                    *at
                }
            };
            Some((last, id, pr))
        })
        .collect();
    out.sort_by_key(|(last, ..)| *last);
    out.into_iter().map(|(_, id, pr)| (id, pr)).collect()
}

/// The merged, unreleased tasks by repository, for the repositories whose
/// releases are due a look.
fn awaiting_release(tasks: &[BoardTask], checked: &Checked, now_ms: i64) -> HashMap<String, Vec<(String, i64)>> {
    let mut out: HashMap<String, Vec<(String, i64)>> = HashMap::new();
    for t in tasks {
        let Some(pr) = t.card.pr.as_ref() else { continue };
        let (Some(merged), true) = (pr.merged_at, pr.released_in.is_empty()) else { continue };
        let Some(r) = parse_pr_ref(&t.card.pr_url, &t.card.repo) else { continue };
        if checked.releases_at.get(&r.repo).is_some_and(|at| now_ms - at < RELEASES_EVERY_MS) {
            continue;
        }
        out.entry(r.repo).or_default().push((t.item.id.clone(), merged));
    }
    out
}

/// The tasks the merge loop looks at: every one not finished, and those
/// finished within the board's window, so a task its agent completed when
/// its PR merged is still followed to its release.
fn watch_list(identity: &Store, now_ms: i64) -> Result<Vec<BoardTask>, crate::backend::storage::StoreError> {
    identity.work_board_list(now_ms - super::FINISHED_WINDOW_MS, 500, now_ms)
}

/// The merge loop: every minute, ask GitHub about the PRs of the tasks
/// that are due a look; record merges and releases, and accept a task in
/// Review whose PR merged. Started once, after [`super::install`].
pub async fn run_merge_loop() {
    let mut checked = Checked::default();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(TICK_MS)).await;
        let Some(ctx) = CONTEXT.get() else { continue };
        // The setting stops a merge accepting a task in Review; merges and
        // releases are still recorded, for the Merged and Released columns.
        let close_on_merge = close_on_merge_enabled(&ctx.config.get_settings().extra);
        let now = agentmux_common::time::now_ms();
        let identity = ctx.identity.clone();
        let tasks = tokio::task::spawn_blocking(move || watch_list(&identity, now)).await;
        let Ok(Ok(tasks)) = tasks else { continue };
        let live: std::collections::HashSet<&str> = tasks.iter().map(|t| t.item.id.as_str()).collect();
        checked.at.retain(|id, _| live.contains(id.as_str()));
        let mut changed = false;
        for (id, pr) in due(&tasks, &checked, now) {
            match get(&ctx.http, &format!("/repos/{}/pulls/{}", pr.repo, pr.number), now).await {
                Ok(v) => {
                    let state = pr_state(&v);
                    checked.at.insert(id.clone(), (now, state.clone()));
                    if state == Some(PrState::Merged) {
                        let identity = ctx.identity.clone();
                        let (id, pr) = (id.clone(), pr.clone());
                        let when = merged_at(&v).unwrap_or(now);
                        changed |= tokio::task::spawn_blocking(move || {
                            let noted = identity.work_pr_merged(&id, when, now).unwrap_or(false);
                            (close_on_merge && close_merged(&identity, &id, &pr, now)) || noted
                        })
                        .await
                        .unwrap_or(false);
                    }
                }
                // A missing PR, or a repository GitHub doesn't show without a
                // login: only the second sets the whole repository aside.
                Err(Miss::NotFound) => match get(&ctx.http, &format!("/repos/{}", pr.repo), now).await {
                    Ok(_) => {
                        checked.at.insert(id, (now, Some(PrState::Missing)));
                    }
                    Err(Miss::NotFound) => {
                        checked.private_repos.insert(pr.repo.clone(), now);
                    }
                    Err(Miss::Limited) => break,
                    Err(Miss::Other(_)) => {
                        checked.at.insert(id, (now, None));
                    }
                },
                Err(Miss::Limited) => break,
                Err(Miss::Other(e)) => {
                    checked.at.insert(id, (now, None));
                    tracing::debug!(error = %e, "kanban: a PR's state not read");
                }
            }
        }
        for (repo, waiting) in awaiting_release(&tasks, &checked, now) {
            match get(&ctx.http, &format!("/repos/{repo}/releases?per_page=30"), now).await {
                Ok(list) => {
                    checked.releases_at.insert(repo, now);
                    let identity = ctx.identity.clone();
                    changed |= tokio::task::spawn_blocking(move || {
                        let mut any = false;
                        for (id, merged) in waiting {
                            if let Some((tag, published)) = release_containing(&list, merged) {
                                any |= identity.work_pr_released(&id, &tag, published, now).unwrap_or(false);
                            }
                        }
                        any
                    })
                    .await
                    .unwrap_or(false);
                }
                Err(Miss::Limited) => break,
                Err(_) => {
                    checked.releases_at.insert(repo, now);
                }
            }
        }
        if changed {
            publish_changed(&ctx.broker);
        }
    }
}

/// How much earlier than the `gh pr create` srv saw a PR may say it was
/// created and still be taken for that command's: clocks differ.
const CREATED_SLACK_MS: i64 = 5 * 60 * 1000;

/// The PR a just-seen `gh pr create` opened, from a list GitHub gave: the
/// first created since `since_ms` (less [`CREATED_SLACK_MS`]), from `branch`
/// when one is given. An older PR from the same branch (merged long ago, or
/// open before this command) is not this command's.
pub fn pick_opened(list: &Value, branch: Option<&str>, since_ms: i64) -> Option<u64> {
    list.as_array()?.iter().find_map(|p| {
        let created = p.get("created_at")?.as_str()?;
        let created_ms = chrono::DateTime::parse_from_rfc3339(created).ok()?.timestamp_millis();
        if created_ms < since_ms - CREATED_SLACK_MS {
            return None;
        }
        if let Some(b) = branch {
            if p.pointer("/head/ref").and_then(Value::as_str) != Some(b) {
                return None;
            }
        }
        p.get("number")?.as_u64()
    })
}

/// An agent holding `work_id` was seen at `seen_ms` opening a PR from `branch`
/// in `repo` (from `owner`'s fork when `--head` named one): look for it on
/// GitHub a few times as it appears, and link the first found that was
/// created by then. Without `--head`, `gh` may have pushed to a fork of the
/// user's, whose owner srv doesn't know: the last look takes the newest open
/// PRs and matches the branch instead.
pub fn find_opened(work_id: String, repo: String, owner: Option<String>, branch: String, seen_ms: i64) {
    let Some(ctx) = CONTEXT.get() else { return };
    let Some(repo) = github_repo(&repo) else { return };
    let Some((repo_owner, _)) = repo.split_once('/') else { return };
    let implicit_owner = owner.is_none();
    let owner = owner.unwrap_or_else(|| repo_owner.to_string());
    let head: String = url::form_urlencoded::byte_serialize(format!("{owner}:{branch}").as_bytes()).collect();
    let by_head = format!("/repos/{repo}/pulls?head={head}&state=all&per_page=10");
    let newest = format!("/repos/{repo}/pulls?state=open&sort=created&direction=desc&per_page=30");
    tokio::spawn(async move {
        let mut waited = 0;
        for (i, after) in FIND_AFTER_S.iter().enumerate() {
            tokio::time::sleep(std::time::Duration::from_secs(after - waited)).await;
            waited = *after;
            let now = agentmux_common::time::now_ms();
            let mut number = match get(&ctx.http, &by_head, now).await {
                Ok(v) => pick_opened(&v, None, seen_ms),
                Err(Miss::NotFound) => return,
                Err(_) => None,
            };
            if number.is_none() && implicit_owner && i + 1 == FIND_AFTER_S.len() {
                number = match get(&ctx.http, &newest, now).await {
                    Ok(v) => pick_opened(&v, Some(&branch), seen_ms),
                    Err(_) => None,
                };
            }
            if let Some(number) = number {
                let pr = PrRef { repo: repo.clone(), number };
                let identity = ctx.identity.clone();
                let id = work_id.clone();
                if tokio::task::spawn_blocking(move || link(&identity, &id, &pr, now)).await.unwrap_or(false) {
                    publish_changed(&ctx.broker);
                }
                return;
            }
        }
    });
}

/// A hand-in named `pr` while its agent was on `branch` of `repo` (its
/// `origin`): link it if that PR is from that branch of that repository, so a
/// PR the result only mentions (someone else's, one it built on, a fork's
/// same-named branch) isn't taken for the task's own.
pub fn link_if_from(work_id: String, pr: PrRef, repo: String, branch: String) {
    let Some(ctx) = CONTEXT.get() else { return };
    if branch.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let now = agentmux_common::time::now_ms();
        let Ok(v) = get(&ctx.http, &format!("/repos/{}/pulls/{}", pr.repo, pr.number), now).await else { return };
        if !is_from_branch(&v, &repo, &branch) {
            return;
        }
        let identity = ctx.identity.clone();
        if tokio::task::spawn_blocking(move || link(&identity, &work_id, &pr, now)).await.unwrap_or(false) {
            publish_changed(&ctx.broker);
        }
    });
}

/// Whether GitHub's answer for a PR says it is from `branch` of `repo`
/// (`owner/repo`).
pub fn is_from_branch(pr: &Value, repo: &str, branch: &str) -> bool {
    let same_ref = pr.pointer("/head/ref").and_then(Value::as_str).is_some_and(|r| !branch.is_empty() && r == branch);
    let same_repo = pr
        .pointer("/head/repo/full_name")
        .and_then(Value::as_str)
        .is_some_and(|r| !repo.is_empty() && r.eq_ignore_ascii_case(repo));
    same_ref && same_repo
}

#[cfg(test)]
#[path = "prs_tests.rs"]
mod tests;
