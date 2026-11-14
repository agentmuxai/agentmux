// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "The review of PR N for agent A: approved, at T" and "the checks of PR N: failing": the `review-flow`,
//! `checks-flow` and `merge-flow` events and their short lists of recent ones, for the review bot and CI nodes
//! of the Atlas agent map (spec `SPEC_REVIEW_AND_CHECKS_FLOW_EVENTS_2026_11_14.md`).
//!
//! They carry ids, numbers, states and times only. A repository is its numeric id, never a name; there is no
//! field for a title, a branch, a URL, a review body, a log or a login, so none can leak through them.
//!
//! The data is a small `flow` object the relay sends beside a review, CI or merge notice. It is not part of
//! the notice's signature, so it is advisory and applied only with the notice it belongs to: that notice must
//! have verified and be fresh, and the object's PR number must be the one in the signed text's first line
//! (see [`accept`]). The recent lists live in memory only and are read with `GET /api/v1/pr-flow`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;

use super::mps::Broker;
use super::mps_events::{ChecksFlow, MergeFlow, ReviewFlow};

/// How many recent events of each kind are kept.
pub const RECENT_MAX: usize = 100;

/// The longest a state name is: it is a token of lower-case letters and `_`, so a state this version does not
/// name (`pending`, `requested`, `closed`) is carried, not refused.
const STATE_MAX: usize = 24;

/// Characters of the head commit kept.
const HEAD_KEPT: usize = 12;

/// The largest whole number a JavaScript number holds exactly (2^53 - 1).
const SAFE_MAX: u64 = (1 << 53) - 1;

/// A pull request by GitHub's own numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PrRef {
    #[ts(type = "number")]
    pub repo_id: u64,
    #[ts(type = "number")]
    pub number: u64,
}

/// What kind of reviewer: coarse, never a login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "lowercase")]
pub enum PrFlowBy {
    /// The review bot (muxreview).
    Bot,
    /// The Codex reviewer.
    Codex,
    /// A person.
    Person,
}

/// A review of a PR, for the agent it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReviewFlowEvent {
    /// The relay message's id: a redelivery is the same event.
    pub id: String,
    /// When this srv accepted the notice, in milliseconds since 1970.
    #[ts(type = "number")]
    pub at_ms: u64,
    /// The local agent the notice was delivered to: the PR's own agent.
    pub agent: String,
    pub pr: PrRef,
    /// `approved`, `changes_requested` or `commented`; any other well-formed name is kept and drawn neutral.
    #[ts(type = "\"approved\" | \"changes_requested\" | \"commented\" | (string & {})")]
    pub state: String,
    pub by: PrFlowBy,
    /// The first 12 hex characters of the reviewed commit, when the notice has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
}

/// The checks of a PR's head commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ChecksFlowEvent {
    pub id: String,
    #[ts(type = "number")]
    pub at_ms: u64,
    pub agent: String,
    pub pr: PrRef,
    /// `passing` or `failing`; any other well-formed name (`pending`) is kept and drawn neutral.
    #[ts(type = "\"passing\" | \"failing\" | (string & {})")]
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
}

/// The end of a PR's lines on the map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MergeFlowEvent {
    pub id: String,
    #[ts(type = "number")]
    pub at_ms: u64,
    pub agent: String,
    pub pr: PrRef,
    /// `merged`; any other well-formed name (`closed`) is kept and drawn neutral.
    #[ts(type = "\"merged\" | (string & {})")]
    pub state: String,
}

/// The bot's own state as this srv sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PrFlowBot {
    /// When the newest accepted notice of any kind came; `null` before the first since this srv started.
    #[ts(type = "number | null")]
    pub last_notice_at_ms: Option<u64>,
}

/// The answer of `GET /api/v1/pr-flow`: the recent events of each kind, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PrFlowSnapshot {
    pub review: Vec<ReviewFlowEvent>,
    pub checks: Vec<ChecksFlowEvent>,
    pub merge: Vec<MergeFlowEvent>,
    pub bot: PrFlowBot,
}

/// A notice's `flow` object that passed every check, as the event it makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    Review(ReviewFlowEvent),
    Checks(ChecksFlowEvent),
    Merge(MergeFlowEvent),
}

/// What the call site knows of one pulled notice.
pub struct Notice<'a> {
    /// The relay message's id.
    pub id: &'a str,
    /// The local agent it was delivered to.
    pub agent: &'a str,
    /// The signed text: its first line names the PR.
    pub text: &'a str,
    /// Its signature verified under a trusted key and it is fresh (not expired, not forged, not unsigned).
    pub genuine: bool,
}

/// `value` as a token of lower-case letters and `_`, 1 to [`STATE_MAX`] long.
fn token(value: &Value) -> Option<String> {
    let s = value.as_str()?;
    let ok = (1..=STATE_MAX).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    ok.then(|| s.to_string())
}

/// `value` as a positive whole number a JavaScript number holds exactly.
fn positive(value: &Value) -> Option<u64> {
    value.as_u64().filter(|n| (1..=SAFE_MAX).contains(n))
}

/// The commit in `value`, kept to [`HEAD_KEPT`] characters: 7 to 40 hex characters, or `Err` when it is
/// there and is not.
fn head_of(value: Option<&Value>) -> Result<Option<String>, ()> {
    let Some(value) = value.filter(|v| !v.is_null()) else { return Ok(None) };
    let s = value.as_str().filter(|s| (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())).ok_or(())?;
    Ok(Some(s[..HEAD_KEPT.min(s.len())].to_ascii_lowercase()))
}

/// The reviewer kind in `value`.
fn by_of(value: Option<&Value>) -> Option<PrFlowBy> {
    match value?.as_str()? {
        "bot" => Some(PrFlowBy::Bot),
        "codex" => Some(PrFlowBy::Codex),
        "person" => Some(PrFlowBy::Person),
        _ => None,
    }
}

/// The number after the first `PR #` of `text`'s first line.
fn pr_in_first_line(text: &str) -> Option<u64> {
    let line = text.lines().next()?;
    let rest = &line[line.find("PR #")? + 4..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The event `flow` makes for `notice`, at `at_ms`, or `None`: the notice carries no object, the notice is not
/// genuine and fresh, the object is malformed in any field, or its PR is not the one the signed text names.
///
/// The last check ties the unsigned object to the signed message: a wording change that moves the PR number
/// out of the first line fails closed (no event) instead of drawing a wrong line.
pub fn accept(flow: Option<&Value>, notice: &Notice<'_>, at_ms: u64) -> Option<Accepted> {
    let flow = flow?.as_object().filter(|_| notice.genuine && !notice.id.is_empty() && !notice.agent.is_empty())?;
    let pr = PrRef { repo_id: positive(flow.get("repo_id")?)?, number: positive(flow.get("pr")?)? };
    if pr_in_first_line(notice.text) != Some(pr.number) {
        return None;
    }
    let state = token(flow.get("state")?)?;
    let head = head_of(flow.get("head")).ok()?;
    let (id, agent) = (notice.id.to_string(), notice.agent.to_string());
    match flow.get("kind")?.as_str()? {
        "review" => Some(Accepted::Review(ReviewFlowEvent { id, at_ms, agent, pr, state, by: by_of(flow.get("by"))?, head })),
        "checks" => Some(Accepted::Checks(ChecksFlowEvent { id, at_ms, agent, pr, state, head })),
        "merge" => Some(Accepted::Merge(MergeFlowEvent { id, at_ms, agent, pr, state })),
        _ => None,
    }
}

#[derive(Default)]
struct Recent {
    review: VecDeque<ReviewFlowEvent>,
    checks: VecDeque<ChecksFlowEvent>,
    merge: VecDeque<MergeFlowEvent>,
    last_notice_at_ms: Option<u64>,
}

/// Adds `event` to `recent` unless one with its `id` is already there, dropping the oldest once there are
/// [`RECENT_MAX`]. Whether it was added.
fn remember<T>(recent: &mut VecDeque<T>, event: T, id: impl Fn(&T) -> &str) -> bool {
    if recent.iter().any(|e| id(e) == id(&event)) {
        return false;
    }
    if recent.len() >= RECENT_MAX {
        recent.pop_front();
    }
    recent.push_back(event);
    true
}

impl Recent {
    /// Keeps `accepted`; whether it is new, so it is published once.
    fn keep(&mut self, accepted: &Accepted) -> bool {
        let (added, at_ms) = match accepted {
            Accepted::Review(e) => (remember(&mut self.review, e.clone(), |e| &e.id), e.at_ms),
            Accepted::Checks(e) => (remember(&mut self.checks, e.clone(), |e| &e.id), e.at_ms),
            Accepted::Merge(e) => (remember(&mut self.merge, e.clone(), |e| &e.id), e.at_ms),
        };
        if added {
            self.last_notice_at_ms = Some(self.last_notice_at_ms.map_or(at_ms, |last| last.max(at_ms)));
        }
        added
    }

    fn snapshot(&self) -> PrFlowSnapshot {
        PrFlowSnapshot {
            review: self.review.iter().cloned().collect(),
            checks: self.checks.iter().cloned().collect(),
            merge: self.merge.iter().cloned().collect(),
            bot: PrFlowBot { last_notice_at_ms: self.last_notice_at_ms },
        }
    }
}

struct Flow {
    broker: Arc<Broker>,
    recent: Mutex<Recent>,
}

static FLOW: OnceLock<Flow> = OnceLock::new();

/// Start publishing on `broker` for the rest of the process. The first call wins.
pub fn install(broker: Arc<Broker>) {
    let _ = FLOW.set(Flow { broker, recent: Mutex::default() });
}

impl Flow {
    /// Keeps `accepted` and, when it is new, publishes it as its kind's event.
    fn record(&self, accepted: &Accepted) {
        if !self.recent.lock().unwrap_or_else(|e| e.into_inner()).keep(accepted) {
            return;
        }
        match accepted {
            Accepted::Review(e) => self.broker.publish_event::<ReviewFlow>(e, vec![], 0),
            Accepted::Checks(e) => self.broker.publish_event::<ChecksFlow>(e, vec![], 0),
            Accepted::Merge(e) => self.broker.publish_event::<MergeFlow>(e, vec![], 0),
        }
    }
}

/// Records and publishes `accepted`. Does nothing before [`install`].
pub fn note(accepted: &Accepted) {
    if let Some(flow) = FLOW.get() {
        flow.record(accepted);
    }
}

/// The event `flow` makes for a notice delivered now, if any, to be passed to [`note`] once the notice is
/// accepted. See [`accept`].
pub fn accept_now(flow: Option<&Value>, notice: &Notice<'_>) -> Option<Accepted> {
    accept(flow, notice, agentmux_common::time::now_ms() as u64)
}

/// The recent events, oldest first.
pub fn recent() -> PrFlowSnapshot {
    FLOW.get().map(|f| f.recent.lock().unwrap_or_else(|e| e.into_inner()).snapshot()).unwrap_or_default()
}

#[cfg(test)]
#[path = "pr_flow_tests.rs"]
mod tests;
