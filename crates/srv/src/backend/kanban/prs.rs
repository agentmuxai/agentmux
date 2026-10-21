// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Kanban and GitHub (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md §6.9, D5):
//!
//! - **Finding the PR.** When an agent holding one task opens a PR, the card
//!   gets the PR's address once GitHub lists it, so the card links to it
//!   without the agent naming it. A result that names one PR ("PR #12") does
//!   the same at hand-in.
//! - **Closing on merge.** A task in Review whose PR has merged is accepted:
//!   the merge was the review. Off with the setting `kanban:closeonmerge`.
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

/// When each task's PR was last asked about, and what came back.
#[derive(Default)]
struct Checked {
    at: HashMap<String, (i64, Option<PrState>)>,
    private_repos: HashMap<String, i64>,
}

/// The tasks in Review whose PR is due a look, oldest look first.
fn due(tasks: &[BoardTask], checked: &Checked, now_ms: i64) -> Vec<(String, PrRef)> {
    let mut out: Vec<(i64, String, PrRef)> = tasks
        .iter()
        .filter(|t| t.item.state == work_state::REVIEW)
        .filter_map(|t| Some((t.item.id.clone(), parse_pr_ref(&t.card.pr_url, &t.card.repo)?)))
        .filter(|(_, pr)| checked.private_repos.get(&pr.repo).is_none_or(|at| now_ms - at >= PRIVATE_FOR_MS))
        .filter_map(|(id, pr)| {
            let last = match checked.at.get(&id) {
                None => i64::MIN,
                Some((at, state)) => {
                    let every = if *state == Some(PrState::Closed) { CLOSED_CHECK_EVERY_MS } else { CHECK_EVERY_MS };
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

/// The merge loop: every minute, ask GitHub about the PRs of the tasks in
/// Review that are due a look, and accept those that merged. Started once,
/// after [`super::install`].
pub async fn run_merge_loop() {
    let mut checked = Checked::default();
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(TICK_MS)).await;
        let Some(ctx) = CONTEXT.get() else { continue };
        if !close_on_merge_enabled(&ctx.config.get_settings().extra) {
            continue;
        }
        let now = agentmux_common::time::now_ms();
        let identity = ctx.identity.clone();
        let tasks = tokio::task::spawn_blocking(move || identity.work_board_list(now, 500, now)).await;
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
                        changed |= tokio::task::spawn_blocking(move || close_merged(&identity, &id, &pr, now)).await.unwrap_or(false);
                    }
                }
                Err(Miss::NotFound) => {
                    checked.private_repos.insert(pr.repo.clone(), now);
                }
                Err(Miss::Limited) => break,
                Err(Miss::Other(e)) => {
                    checked.at.insert(id, (now, None));
                    tracing::debug!(error = %e, "kanban: a PR's state not read");
                }
            }
        }
        if changed {
            publish_changed(&ctx.broker);
        }
    }
}

/// An agent holding `work_id` just opened a PR from `branch` in `repo`: look
/// for it on GitHub a few times as it appears, and link the first found.
pub fn find_opened(work_id: String, repo: String, branch: String) {
    let Some(ctx) = CONTEXT.get() else { return };
    let Some(repo) = github_repo(&repo) else { return };
    let Some((owner, _)) = repo.split_once('/') else { return };
    let head: String = url::form_urlencoded::byte_serialize(format!("{owner}:{branch}").as_bytes()).collect();
    let path = format!("/repos/{repo}/pulls?head={head}&state=all&per_page=1");
    tokio::spawn(async move {
        let mut waited = 0;
        for after in FIND_AFTER_S {
            tokio::time::sleep(std::time::Duration::from_secs(after - waited)).await;
            waited = after;
            let now = agentmux_common::time::now_ms();
            let number = match get(&ctx.http, &path, now).await {
                Ok(v) => v.as_array().and_then(|a| a.first()).and_then(|p| p.get("number")).and_then(Value::as_u64),
                Err(Miss::NotFound) => return,
                Err(_) => None,
            };
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

#[cfg(test)]
#[path = "prs_tests.rs"]
mod tests;
