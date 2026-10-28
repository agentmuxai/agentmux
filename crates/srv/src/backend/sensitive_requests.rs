// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The requests for sensitive operations (`SPEC_SUPER_AGENT_2026_10_25.md` §4):
//! who asked for what, what became of it. A request is `asked` until the super
//! agent answers; `approve` makes it `pending` (the countdown runs); the rest
//! are the ways it ends. Kept in memory: a request is a minutes-long exchange,
//! and an AgentMux that restarts mid-way has the requester ask again.
//!
//! This holds the state and its rules only. The `Sensitive` routes
//! (`server/sensitive_handlers.rs`) decide who may file, approve or deny, and the
//! countdown is `sagas/sensitive_window.rs`. Not every way a request ends has a
//! caller yet (a request nobody reviewed arrives with a later step of that
//! spec's §9).
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
    /// The countdown ended and the operation is under way: no longer cancellable.
    Running,
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
    /// Not ended: undecided, counting down or under way.
    pub fn is_open(self) -> bool {
        matches!(self, Status::Asked | Status::Pending | Status::Running)
    }
}

/// An agent that asked for something already asked for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Asker {
    pub uid: String,
    pub slug: String,
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
    pub also_asked_by: Vec<Asker>,
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
            if !open.asked_by(requester_uid) {
                open.also_asked_by.push(Asker { uid: requester_uid.to_string(), slug: requester_slug.to_string() });
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

    /// Nobody reviewed it in time: `asked` becomes `pending` with no approver, and the person's
    /// veto is the only check.
    pub fn fallback(&self, id: &str, now_ms: i64) -> Result<Request, String> {
        self.change(id, |r| {
            if r.status != Status::Asked {
                return Err(not_asked(r));
            }
            r.status = Status::Pending;
            r.decided_by = None;
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

    /// An agent that asked withdraws its own ask while the request is open. The
    /// request goes on for the others that asked; it ends when the last one withdraws.
    pub fn cancel(&self, id: &str, requester_uid: &str, now_ms: i64) -> Result<Request, String> {
        self.change(id, |r| {
            if !r.asked_by(requester_uid) {
                return Err("FORBIDDEN: only an agent that asked can cancel it".to_string());
            }
            if r.status == Status::Running {
                return Err(format!("request {} is already under way", r.id));
            }
            if !r.status.is_open() {
                return Err(format!("request {} has ended ({})", r.id, status_word(r.status)));
            }
            if r.requester_uid != requester_uid {
                r.also_asked_by.retain(|a| a.uid != requester_uid);
            } else if r.also_asked_by.is_empty() {
                r.status = Status::Cancelled;
                r.ended_ms = Some(now_ms);
            } else {
                let next = r.also_asked_by.remove(0);
                r.requester_uid = next.uid;
                r.requester_slug = next.slug;
            }
            Ok(())
        })
    }

    /// The countdown ended: `pending` becomes `running`, atomically, so a
    /// request withdrawn or vetoed meanwhile is not run.
    pub fn start(&self, id: &str) -> Result<Request, String> {
        self.change(id, |r| {
            if r.status != Status::Pending {
                return Err(format!("request {} is {}, not approved", r.id, status_word(r.status)));
            }
            r.status = Status::Running;
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

    /// The open requests for `op` whose fact `key` is `value`.
    pub fn open_matching(&self, op: &str, key: &str, value: &str) -> Vec<Request> {
        self.lock().iter().filter(|r| r.status.is_open() && r.status != Status::Running && r.op == op && r.facts.get(key).is_some_and(|v| v == value)).cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<Request> {
        self.lock().iter().find(|r| r.id == id).cloned()
    }

    /// The requests `viewer_uid` may see: those it asked for, or all of them (the super agent).
    pub fn list(&self, viewer_uid: &str, all: bool) -> Vec<Request> {
        self.lock().iter().filter(|r| all || r.asked_by(viewer_uid)).cloned().collect()
    }

    fn change(&self, id: &str, edit: impl FnOnce(&mut Request) -> Result<(), String>) -> Result<Request, String> {
        let mut all = self.lock();
        let r = all.iter_mut().find(|r| r.id == id).ok_or_else(|| format!("no request {id}"))?;
        edit(r)?;
        Ok(r.clone())
    }
}

impl Request {
    /// The agents that asked for this: the first, then those that joined.
    pub fn askers(&self) -> Vec<&str> {
        std::iter::once(self.requester_uid.as_str()).chain(self.also_asked_by.iter().map(|a| a.uid.as_str())).collect()
    }

    /// Whether `uid` asked for this, first or by joining.
    pub fn asked_by(&self, uid: &str) -> bool {
        self.requester_uid == uid || self.also_asked_by.iter().any(|a| a.uid == uid)
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
