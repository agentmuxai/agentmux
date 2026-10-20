// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agents' requests for a new Vault entry, waiting for the person
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.4).
//!
//! `Vault` `request` asks here; the pane shows each request and the person
//! fills the entry in or dismisses it, through the host. The agent hears the
//! new entry's id, never its value. An agent can ask; it can never answer.

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

#[derive(Default)]
pub struct Requests {
    pending: Mutex<Vec<(VaultRequest, oneshot::Sender<Answer>)>>,
}

pub fn requests() -> &'static Requests {
    static REQUESTS: OnceLock<Requests> = OnceLock::new();
    REQUESTS.get_or_init(Requests::default)
}

/// At most this many requests wait at once; the oldest is declined.
const MAX_PENDING: usize = 16;
const MAX_AGE_MS: u64 = 60 * 60 * 1000;

impl Requests {
    pub fn ask(&self, req: VaultRequest) -> oneshot::Receiver<Answer> {
        let (tx, rx) = oneshot::channel();
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        if pending.len() >= MAX_PENDING {
            let (_, oldest) = pending.remove(0);
            let _ = oldest.send(Answer::Declined);
        }
        pending.push((req, tx));
        rx
    }

    /// The person's answer; false if nothing waits under `id`.
    pub fn answer(&self, id: &str, answer: Answer) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let Some(i) = pending.iter().position(|(r, _)| r.id == id) else {
            return false;
        };
        let (_, tx) = pending.remove(i);
        let _ = tx.send(answer);
        true
    }

    pub fn get(&self, id: &str) -> Option<VaultRequest> {
        let pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.iter().find(|(r, _)| r.id == id).map(|(r, _)| r.clone())
    }

    /// The requests still open. One stays after its agent stops waiting (the
    /// agent finds the entry with `list` later), until it is an hour old.
    pub fn list(&self) -> Vec<VaultRequest> {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let now = agentmux_common::time::now_ms_u64();
        pending.retain(|(r, _)| now.saturating_sub(r.requested_ms) < MAX_AGE_MS);
        pending.iter().map(|(r, _)| r.clone()).collect()
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
            requested_ms: agentmux_common::time::now_ms_u64(),
        }
    }

    #[tokio::test]
    async fn an_answer_reaches_the_waiting_agent_once() {
        let r = Requests::default();
        let rx = r.ask(req("q1"));
        assert_eq!(r.list().len(), 1);
        assert!(r.answer("q1", Answer::Created("api-key-stripe".into())));
        assert!(!r.answer("q1", Answer::Declined), "answered already");
        assert_eq!(rx.await.unwrap(), Answer::Created("api-key-stripe".into()));
        assert!(r.list().is_empty());
    }

    #[test]
    fn a_request_outlives_its_wait_but_not_an_hour() {
        let r = Requests::default();
        drop(r.ask(req("q1")));
        assert_eq!(r.list().len(), 1, "the person can still answer it");
        assert!(r.answer("q1", Answer::Declined), "answering a request nobody waits on is fine");
        let mut old = req("q2");
        old.requested_ms -= MAX_AGE_MS + 1;
        drop(r.ask(old));
        assert!(r.list().is_empty());
    }
}
