// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agents' requests for a new Vault entry, waiting for the person
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.4).
//!
//! `Vault` `request` asks here; the pane shows each request and the person
//! fills the entry in or dismisses it, through the host. The agent hears the
//! new entry's id, never its value. An agent can ask; it can never answer.
//!
//! An agent's session can end while it waits (a restart, a resume, an upgrade),
//! and the agent then asks again. The same verified agent asking for the same
//! kind and label is that retry, so it joins the request already open, or gets
//! the answer the person already gave, rather than opening a second form.

use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct VaultRequest {
    pub id: String,
    /// The agent that asked, as its verified identity names it.
    pub agent: String,
    pub kind: String,
    pub label: String,
    /// The agent's reason, shown to the person.
    pub why: String,
    #[ts(type = "number")]
    pub requested_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The person saved it as this entry.
    Created(String),
    Declined,
}

/// What asking gave: a request to wait on, or the answer an earlier ask of the
/// same entry already got. Each carries the request's id.
#[derive(Debug)]
pub enum Asked {
    /// A new request (`new`), or the one already open for this agent, kind and label.
    Waiting { request_id: String, new: bool, answer: oneshot::Receiver<Answer> },
    Answered { request_id: String, answer: Answer },
}

struct Pending {
    req: VaultRequest,
    /// Whether a retry may join it: only a verified agent's request.
    joinable: bool,
    waiters: Vec<oneshot::Sender<Answer>>,
}

struct Answered {
    req: VaultRequest,
    answer: Answer,
    at_ms: u64,
}

#[derive(Default)]
pub struct Requests {
    pending: Mutex<Vec<Pending>>,
    answered: Mutex<Vec<Answered>>,
}

pub fn requests() -> &'static Requests {
    static REQUESTS: OnceLock<Requests> = OnceLock::new();
    REQUESTS.get_or_init(Requests::default)
}

/// At most this many requests wait at once; the oldest is declined.
const MAX_PENDING: usize = 16;
const MAX_AGE_MS: u64 = 60 * 60 * 1000;
/// Most answers kept for a retry to find.
const MAX_ANSWERED: usize = 64;

fn same(a: &VaultRequest, b: &VaultRequest) -> bool {
    a.agent == b.agent && a.kind == b.kind && a.label.trim().to_lowercase() == b.label.trim().to_lowercase()
}

fn now() -> u64 {
    agentmux_common::time::now_ms_u64()
}

impl Requests {
    /// Asks the person for `req`. `verified` says the agent's identity was proven:
    /// only then is an ask for the same kind and label taken as a retry, since
    /// every unverified caller shares one name.
    pub fn ask(&self, req: VaultRequest, verified: bool) -> Asked {
        if verified {
            let mut answered = self.answered.lock().unwrap_or_else(|p| p.into_inner());
            answered.retain(|a| now().saturating_sub(a.at_ms) < MAX_AGE_MS);
            if let Some(a) = answered.iter().rev().find(|a| same(&a.req, &req)) {
                return Asked::Answered { request_id: a.req.id.clone(), answer: a.answer.clone() };
            }
        }
        let (tx, rx) = oneshot::channel();
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let now = now();
        pending.retain(|p| now.saturating_sub(p.req.requested_ms) < MAX_AGE_MS);
        if verified {
            if let Some(open) = pending.iter_mut().find(|p| p.joinable && same(&p.req, &req)) {
                open.waiters.push(tx);
                return Asked::Waiting { request_id: open.req.id.clone(), new: false, answer: rx };
            }
        }
        if pending.len() >= MAX_PENDING {
            let oldest = pending.remove(0);
            for w in oldest.waiters {
                let _ = w.send(Answer::Declined);
            }
        }
        let request_id = req.id.clone();
        pending.push(Pending { req, joinable: verified, waiters: vec![tx] });
        Asked::Waiting { request_id, new: true, answer: rx }
    }

    /// The person's answer; false if nothing waits under `id`. Every agent waiting
    /// on it hears it, and a retry within the hour finds it.
    pub fn answer(&self, id: &str, answer: Answer) -> bool {
        let open = {
            let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            let Some(i) = pending.iter().position(|p| p.req.id == id) else {
                return false;
            };
            pending.remove(i)
        };
        for w in open.waiters {
            let _ = w.send(answer.clone());
        }
        if open.joinable {
            let mut answered = self.answered.lock().unwrap_or_else(|p| p.into_inner());
            if answered.len() >= MAX_ANSWERED {
                answered.remove(0);
            }
            answered.push(Answered { req: open.req, answer, at_ms: now() });
        }
        true
    }

    pub fn get(&self, id: &str) -> Option<VaultRequest> {
        let pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.iter().find(|p| p.req.id == id).map(|p| p.req.clone())
    }

    /// The requests still open. One stays after its agent stops waiting (the
    /// agent finds the entry with `list`, or by asking again), until it is an hour old.
    pub fn list(&self) -> Vec<VaultRequest> {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let now = now();
        pending.retain(|p| now.saturating_sub(p.req.requested_ms) < MAX_AGE_MS);
        pending.iter().map(|p| p.req.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str) -> VaultRequest {
        VaultRequest {
            id: id.into(),
            agent: "Agent1".into(),
            kind: "api-key".into(),
            label: "Stripe".into(),
            why: "tests".into(),
            requested_ms: now(),
        }
    }

    fn waiting(a: Asked) -> (String, bool, oneshot::Receiver<Answer>) {
        match a {
            Asked::Waiting { request_id, new, answer } => (request_id, new, answer),
            other => panic!("expected a wait, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_answer_reaches_the_waiting_agent_once() {
        let r = Requests::default();
        let (id, new, rx) = waiting(r.ask(req("q1"), true));
        assert!(new);
        assert_eq!(id, "q1");
        assert_eq!(r.list().len(), 1);
        assert!(r.answer("q1", Answer::Created("api-key-stripe".into())));
        assert!(!r.answer("q1", Answer::Declined), "answered already");
        assert_eq!(rx.await.unwrap(), Answer::Created("api-key-stripe".into()));
        assert!(r.list().is_empty());
    }

    #[test]
    fn a_request_outlives_its_wait_but_not_an_hour() {
        let r = Requests::default();
        drop(r.ask(req("q1"), true));
        assert_eq!(r.list().len(), 1, "the person can still answer it");
        assert!(r.answer("q1", Answer::Declined), "answering a request nobody waits on is fine");
        let mut old = req("q2");
        old.requested_ms -= MAX_AGE_MS + 1;
        drop(r.ask(old, true));
        assert!(r.list().is_empty());
    }

    // QTest, 2026-10-27: three asks for one key, each cut off when the session ended,
    // opened three forms. A retry now joins the one already open.
    #[tokio::test]
    async fn asking_again_for_the_same_entry_joins_the_open_request() {
        let r = Requests::default();
        let (first, _, rx1) = waiting(r.ask(req("q1"), true));
        let mut again = req("q2");
        again.label = "  stripe ".into();
        let (second, new, rx2) = waiting(r.ask(again, true));
        assert_eq!(second, first);
        assert!(!new);
        assert_eq!(r.list().len(), 1, "no second form");
        r.answer("q1", Answer::Created("api-key-stripe".into()));
        assert_eq!(rx1.await.unwrap(), Answer::Created("api-key-stripe".into()));
        assert_eq!(rx2.await.unwrap(), Answer::Created("api-key-stripe".into()));
    }

    #[test]
    fn asking_again_after_the_answer_gets_the_answer() {
        let r = Requests::default();
        drop(r.ask(req("q1"), true));
        r.answer("q1", Answer::Created("api-key-stripe".into()));
        match r.ask(req("q3"), true) {
            Asked::Answered { request_id, answer } => {
                assert_eq!(request_id, "q1");
                assert_eq!(answer, Answer::Created("api-key-stripe".into()));
            }
            other => panic!("expected the earlier answer, got {other:?}"),
        }
        assert!(r.list().is_empty(), "nothing new is opened");
    }

    #[test]
    fn another_entry_or_another_agent_is_a_new_request() {
        let r = Requests::default();
        drop(r.ask(req("q1"), true));
        let mut other_label = req("q2");
        other_label.label = "Stripe live".into();
        let mut other_agent = req("q3");
        other_agent.agent = "Agent2".into();
        let mut other_kind = req("q4");
        other_kind.kind = "login".into();
        for q in [other_label, other_agent, other_kind] {
            assert!(waiting(r.ask(q, true)).1, "a different entry gets its own request");
        }
        assert_eq!(r.list().len(), 4);
    }

    #[test]
    fn unverified_callers_never_join_or_reuse() {
        let r = Requests::default();
        drop(r.ask(req("q1"), false));
        assert!(waiting(r.ask(req("q2"), false)).1);
        assert!(waiting(r.ask(req("q3"), true)).1, "a verified agent doesn't join an unverified request");
        r.answer("q1", Answer::Created("x".into()));
        assert!(matches!(r.ask(req("q5"), false), Asked::Waiting { new: true, .. }), "an unverified answer isn't kept for reuse");
    }
}
