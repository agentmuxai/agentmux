// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The requests for sensitive operations (`SPEC_SUPER_AGENT_2026_10_25.md` §4):
//! who asked for what, what became of it. A request is `asked` until the super
//! agent answers; `approve` makes it `pending` (the countdown runs); the rest
//! are the ways it ends. Kept in memory: a request is a minutes-long exchange,
//! and an AgentMux that restarts mid-way has the requester ask again.
//!
//! This holds the state and its rules only. Who may approve, the countdown and
//! the messages arrive with the later steps of that spec's §9, so nothing
//! outside the tests calls this yet.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};

use crate::backend::sensitive_ops::{self, OpSpec};

/// Requests one agent may have open (asked or pending) at once.
pub const MAX_OPEN_PER_REQUESTER: usize = 8;
/// Requests open at once, all agents together.
pub const MAX_OPEN: usize = 200;
/// A finished request stays answerable this long, then is dropped.
const KEEP_FINISHED_MS: i64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Waiting for the super agent.
    Asked,
    /// Approved: the countdown runs and the person can still stop it.
    Pending,
    Denied,
    /// The requester withdrew it.
    Cancelled,
    /// The person stopped it in the window.
    Vetoed,
    Done,
    Failed,
    /// Nobody answered in time.
    Expired,
}

impl Status {
    /// Asked or pending: still undecided or counting down.
    pub fn is_open(self) -> bool {
        matches!(self, Status::Asked | Status::Pending)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Request {
    pub id: String,
    pub requester_uid: String,
    pub requester_slug: String,
    pub op: String,
    /// The checked facts the banner is built from.
    pub facts: BTreeMap<String, String>,
    /// What the banner says, from `facts`.
    pub summary: String,
    /// The requester's own words, cleaned. Shown as theirs, never as the operator's.
    pub reason: String,
    pub status: Status,
    /// The super agent that approved or denied it.
    pub decided_by: Option<String>,
    /// Why it was denied.
    pub deny_reason: String,
    /// Other agents that asked for the same thing while it was open.
    pub also_asked_by: Vec<String>,
    pub created_ms: i64,
    /// When it was approved, or ended.
    pub decided_ms: Option<i64>,
    pub ended_ms: Option<i64>,
}

/// What filing a request came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filed {
    pub id: String,
    /// It joined a request for the same thing that was already open, whose clock
    /// keeps running.
    pub joined: bool,
}

#[derive(Default)]
pub struct Registry {
    requests: Mutex<Vec<Request>>,
}

static GLOBAL: LazyLock<Registry> = LazyLock::new(Registry::default);

/// The registry of this srv.
pub fn registry() -> &'static Registry {
    &GLOBAL
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Request>> {
        self.requests.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// File a request for `op` from the agent `requester_uid`/`requester_slug`.
    /// A second request for the same operation and facts while one is open joins it.
    pub fn request(
        &self,
        requester_uid: &str,
        requester_slug: &str,
        op: &str,
        given: &BTreeMap<String, String>,
        reason: &str,
        now_ms: i64,
    ) -> Result<Filed, String> {
        let spec: &OpSpec = sensitive_ops::spec(op).ok_or_else(|| {
            let known: Vec<&str> = sensitive_ops::OPS.iter().filter(|o| o.reviewed).map(|o| o.op).collect();
            format!("INVALID_ARG: unknown operation '{}'; one of: {}", op.chars().take(40).collect::<String>(), known.join(", "))
        })?;
        if !spec.reviewed {
            return Err(format!("INVALID_ARG: {} has its own window and no super agent in front of it", spec.op));
        }
        let facts = spec.check_facts(given)?;
        let mut all = self.lock();
        prune(&mut all, now_ms);
        if let Some(open) = all.iter_mut().find(|r| r.status.is_open() && r.op == spec.op && r.facts == facts) {
            if open.requester_uid != requester_uid && !open.also_asked_by.iter().any(|s| s == requester_slug) {
                open.also_asked_by.push(requester_slug.to_string());
            }
            return Ok(Filed { id: open.id.clone(), joined: true });
        }
        if all.iter().filter(|r| r.status.is_open() && r.requester_uid == requester_uid).count() >= MAX_OPEN_PER_REQUESTER {
            return Err(format!("LIMIT: you have {MAX_OPEN_PER_REQUESTER} requests open; cancel or wait for one"));
        }
        if all.iter().filter(|r| r.status.is_open()).count() >= MAX_OPEN {
            return Err("LIMIT: too many sensitive requests are open; try again shortly".to_string());
        }
        let id = uuid::Uuid::new_v4().to_string();
        all.push(Request {
            id: id.clone(),
            requester_uid: requester_uid.to_string(),
            requester_slug: requester_slug.to_string(),
            op: spec.op.to_string(),
            summary: spec.summary(&facts),
            facts,
            reason: crate::sagas::pending_shutdown::clean_reason(reason),
            status: Status::Asked,
            decided_by: None,
            deny_reason: String::new(),
            also_asked_by: Vec::new(),
            created_ms: now_ms,
            decided_ms: None,
            ended_ms: None,
        });
        Ok(Filed { id, joined: false })
    }

    /// The super agent approves: `asked` becomes `pending`.
    pub fn approve(&self, id: &str, approver_slug: &str, now_ms: i64) -> Result<Request, String> {
        self.change(id, |r| {
            if r.status != Status::Asked {
                return Err(not_asked(r));
            }
            r.status = Status::Pending;
            r.decided_by = Some(approver_slug.to_string());
            r.decided_ms = Some(now_ms);
            Ok(())
        })
    }

    /// The super agent denies: `asked` ends, with its reason for the requester.
    pub fn deny(&self, id: &str, approver_slug: &str, reason: &str, now_ms: i64) -> Result<Request, String> {
        self.change(id, |r| {
            if r.status != Status::Asked {
                return Err(not_asked(r));
            }
            r.status = Status::Denied;
            r.decided_by = Some(approver_slug.to_string());
            r.deny_reason = crate::sagas::pending_shutdown::clean_reason(reason);
            r.decided_ms = Some(now_ms);
            r.ended_ms = Some(now_ms);
            Ok(())
        })
    }

    /// The requester withdraws its own request while it is open.
    pub fn cancel(&self, id: &str, requester_uid: &str, now_ms: i64) -> Result<Request, String> {
        self.change(id, |r| {
            if r.requester_uid != requester_uid {
                return Err("FORBIDDEN: only the agent that asked can cancel it".to_string());
            }
            if !r.status.is_open() {
                return Err(format!("request {} has ended ({})", r.id, status_word(r.status)));
            }
            r.status = Status::Cancelled;
            r.ended_ms = Some(now_ms);
            Ok(())
        })
    }

    /// End an open request as `outcome` (vetoed, done, failed or expired).
    pub fn finish(&self, id: &str, outcome: Status, now_ms: i64) -> Result<Request, String> {
        if !matches!(outcome, Status::Vetoed | Status::Done | Status::Failed | Status::Expired) {
            return Err(format!("{} is not a way a request ends", status_word(outcome)));
        }
        self.change(id, |r| {
            if !r.status.is_open() {
                return Err(format!("request {} has ended ({})", r.id, status_word(r.status)));
            }
            r.status = outcome;
            r.ended_ms = Some(now_ms);
            Ok(())
        })
    }

    pub fn get(&self, id: &str) -> Option<Request> {
        self.lock().iter().find(|r| r.id == id).cloned()
    }

    /// The requests `viewer_uid` may see: its own, or all of them (the super agent).
    pub fn list(&self, viewer_uid: &str, all: bool) -> Vec<Request> {
        self.lock().iter().filter(|r| all || r.requester_uid == viewer_uid).cloned().collect()
    }

    fn change(&self, id: &str, edit: impl FnOnce(&mut Request) -> Result<(), String>) -> Result<Request, String> {
        let mut all = self.lock();
        let r = all.iter_mut().find(|r| r.id == id).ok_or_else(|| format!("no request {id}"))?;
        edit(r)?;
        Ok(r.clone())
    }
}

fn not_asked(r: &Request) -> String {
    format!("request {} is {}, not waiting for an answer", r.id, status_word(r.status))
}

fn status_word(s: Status) -> String {
    serde_json::to_value(s).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// Drop what finished long enough ago.
fn prune(all: &mut Vec<Request>, now_ms: i64) {
    all.retain(|r| r.ended_ms.is_none_or(|ended| now_ms - ended < KEEP_FINISHED_MS));
}

#[cfg(test)]
#[path = "sensitive_requests_tests.rs"]
mod tests;
