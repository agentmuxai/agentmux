// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A change to a pull request, as it reaches this install (the Kanban and GitHub spec, §5): the intake that
//! trusts it or not, matches it to a card and writes the card's list.
//!
//! An event is a claim until its sender is proven. It is applied only when **both** hold: the signed sender is
//! the GitHub integration's own service identity, and the signed text itself says `"kind":"pr_event"` (the
//! signature covers the text, not a row's own kind). Anything else is dropped and counted, never applied. The
//! repository's owner must be one the install already trusts for pull request notices. A proven event can still
//! carry a forged hint (any agent can write any agent tag), so a hint alone only ever **shows** (§10): a pull
//! request that is not already on the card, and was not opened by AgentMux's own identity, is kept as an
//! unmatched line and never linked.
//!
//! The relay delivers it as the field of a signed notice; wiring that in is the relay half's seam. This is what
//! happens once an event is read from the verified text. The kinds of event it applies today are the pull
//! request's own states (`opened`, `reopened`, `closed`, `merged`); `review`, `checks` and `release` are read
//! and set aside until the card shows them.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::Value;

use crate::backend::storage::store::Store;
use crate::backend::storage::work_card_prs::{github_repo, CardPrState, PrReport, PrSource};
use crate::backend::storage::work_pr_match::{AgentRef, Matched, TaskRef, Why};

/// How long a delivery id is remembered, so a replay is ignored.
const SEEN_FOR_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrAction {
    Opened,
    Reopened,
    Closed,
    Merged,
    Review,
    Checks,
    Release,
}

/// One change to a pull request. Ids, numbers and states only: no title, body, branch, diff or author.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PrEvent {
    /// `owner/name`.
    pub repo: String,
    #[serde(default)]
    pub number: u64,
    pub action: PrAction,
    /// The pull request's head commit, when the event says.
    #[serde(default)]
    pub head: Option<String>,
    /// GitHub's own time for the change: epoch milliseconds, or an RFC 3339 string.
    #[serde(default)]
    pub updated_at: Value,
    /// One per change, so a replay is ignored.
    #[serde(default)]
    pub delivery: String,
    /// The task reference read from the pull request (`#12`, `board#12`).
    #[serde(default)]
    pub task_ref: Option<String>,
    /// The agent tag read from the pull request: an agent id.
    #[serde(default)]
    pub agent: Option<String>,
    /// Whether AgentMux's own GitHub identity opened the pull request.
    #[serde(default)]
    pub by_agentmux: bool,
}

/// The `pr_event` a verified signed text carries, or `None` when its kind is not that or it is malformed.
pub fn parse_pr_event(text: &str) -> Option<PrEvent> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v.get("kind").and_then(Value::as_str) != Some("pr_event") {
        return None;
    }
    serde_json::from_value(v).ok()
}

fn time_ms(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis()),
        _ => None,
    }
}

/// What the intake did with an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It reached a card's list.
    Applied { work_id: String, changed: bool, newly_merged: bool },
    /// No card: kept on the unmatched list for a person to link.
    Unmatched(Why),
    /// Read and set aside: a kind the card does not show yet.
    Ignored(&'static str),
    /// Not applied, and counted.
    Dropped(&'static str),
}

/// What the install trusts the sender and the repository on.
pub struct Trust<'a> {
    /// The signed sender is the GitHub integration's own service identity.
    pub from_integration: bool,
    /// The organisations whose pull request notices this install takes (lower case).
    pub trusted_owners: &'a [String],
}

/// The intake: remembers the deliveries it saw and what it dropped.
#[derive(Default)]
pub struct Intake {
    seen: Mutex<HashMap<String, i64>>,
    dropped: Mutex<HashMap<&'static str, u64>>,
}

impl Intake {
    fn drop_with(&self, why: &'static str) -> Outcome {
        *self.dropped.lock().unwrap_or_else(|e| e.into_inner()).entry(why).or_default() += 1;
        Outcome::Dropped(why)
    }

    /// How many events were dropped, by reason.
    pub fn dropped(&self) -> HashMap<&'static str, u64> {
        self.dropped.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A delivery id seen in the last hour is a replay; otherwise it is remembered.
    fn replay(&self, delivery: &str, now_ms: i64) -> bool {
        if delivery.is_empty() {
            return false;
        }
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|_, at| now_ms - *at < SEEN_FOR_MS);
        seen.insert(delivery.to_string(), now_ms).is_some()
    }

    /// Apply one event. `agent_of` says whether an agent id is an agent of this computer.
    pub fn apply(
        &self,
        identity: &Store,
        ev: &PrEvent,
        trust: &Trust<'_>,
        agent_of: &dyn Fn(&str) -> Option<AgentRef>,
        now_ms: i64,
    ) -> Outcome {
        if !trust.from_integration {
            return self.drop_with("not from the integration");
        }
        let Some(repo) = github_repo(&ev.repo).filter(|_| ev.number > 0) else { return self.drop_with("not a pull request") };
        let owner = repo.split('/').next().unwrap_or("");
        if !trust.trusted_owners.iter().any(|o| o.eq_ignore_ascii_case(owner)) {
            return self.drop_with("untrusted owner");
        }
        if self.replay(&ev.delivery, now_ms) {
            return self.drop_with("replayed delivery");
        }
        let state = match ev.action {
            PrAction::Opened | PrAction::Reopened => CardPrState::Open,
            PrAction::Closed => CardPrState::Closed,
            PrAction::Merged => CardPrState::Merged,
            PrAction::Review | PrAction::Checks | PrAction::Release => return Outcome::Ignored("not shown on the card yet"),
        };
        let updated_at = time_ms(&ev.updated_at).unwrap_or(now_ms);
        let agent = ev.agent.as_deref().and_then(agent_of);
        let task_ref = ev.task_ref.as_deref().and_then(TaskRef::parse);
        let Ok(matched) = identity.work_pr_match(&repo, ev.number, task_ref.as_ref(), agent.as_ref(), now_ms) else {
            return self.drop_with("the store did not answer");
        };
        let why = match matched {
            Matched::Card(work_id) => {
                let on_card = identity.work_card_prs(&work_id).map(|l| l.iter().any(|p| p.repo == repo && p.number == ev.number)).unwrap_or(false);
                if on_card || ev.by_agentmux {
                    let report = PrReport {
                        repo: repo.clone(),
                        number: ev.number,
                        state,
                        head: ev.head.clone(),
                        opened_at: (ev.action == PrAction::Opened).then_some(updated_at),
                        merged_at: (state == CardPrState::Merged).then_some(updated_at),
                        updated_at,
                    };
                    return match identity.work_card_pr_apply(&work_id, &report, PrSource::Event, now_ms) {
                        Ok(a) => Outcome::Applied { work_id, changed: a.changed, newly_merged: a.newly_merged },
                        Err(_) => self.drop_with("the store did not answer"),
                    };
                }
                // A pull request from outside that names one of our task numbers: shown, never linked.
                Why::Outsider
            }
            Matched::Unmatched(why) => why,
        };
        // Only a pull request whose tag names an agent of this computer is kept; anything else is no card's here.
        match agent {
            Some(a) => {
                let _ = identity.work_pr_unmatched_note(
                    &repo,
                    ev.number,
                    state.as_str(),
                    &a.id,
                    ev.task_ref.as_deref().unwrap_or(""),
                    why,
                    ev.head.as_deref().unwrap_or(""),
                    updated_at,
                    now_ms,
                );
                Outcome::Unmatched(why)
            }
            None => self.drop_with("no card of this computer"),
        }
    }
}

#[cfg(test)]
#[path = "pr_events_tests.rs"]
mod tests;
