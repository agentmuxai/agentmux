// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's use of a Vault entry, waiting for the person to allow it
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §6.4).
//!
//! The agent asks (`Browser` `fill` with `vault`); every window shows the person
//! the entry, the site and the field; their answer reaches srv only through the
//! host's Vault route. An agent can ask; it can never answer.

use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct UseRequest {
    pub id: String,
    /// The agent, as its verified identity names it.
    pub agent: String,
    pub entry_id: String,
    pub entry_label: String,
    /// What is being filled: "Card number", "Social Security number".
    pub what: String,
    /// The site the field is on (its frame's origin).
    pub origin: String,
    /// The field, as a person reads it.
    pub field: String,
    /// Whether "Always on this site" is offered: only when the field looks like
    /// what the entry is.
    pub offer_always: bool,
    /// Why the person is being asked: it is a new site, or the field doesn't look
    /// like what the entry is.
    pub reason: String,
    #[ts(type = "number")]
    pub requested_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Once,
    Always,
    Deny,
}

impl Decision {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "once" => Some(Self::Once),
            "always" => Some(Self::Always),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct Uses {
    pending: Mutex<Vec<(UseRequest, oneshot::Sender<Decision>)>>,
}

pub fn uses() -> &'static Uses {
    static USES: OnceLock<Uses> = OnceLock::new();
    USES.get_or_init(Uses::default)
}

/// At most this many wait at once; the oldest is denied.
const MAX_PENDING: usize = 16;

impl Uses {
    pub fn ask(&self, req: UseRequest) -> oneshot::Receiver<Decision> {
        let (tx, rx) = oneshot::channel();
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        if pending.len() >= MAX_PENDING {
            let (_, oldest) = pending.remove(0);
            let _ = oldest.send(Decision::Deny);
        }
        pending.push((req, tx));
        rx
    }

    /// The request, for the host's answer to check against.
    pub fn get(&self, id: &str) -> Option<UseRequest> {
        let pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.iter().find(|(r, _)| r.id == id).map(|(r, _)| r.clone())
    }

    /// The person's answer; false if nothing waits under `id` (answered, or the
    /// agent stopped waiting).
    pub fn answer(&self, id: &str, decision: Decision) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let Some(i) = pending.iter().position(|(r, _)| r.id == id) else { return false };
        let (_, tx) = pending.remove(i);
        tx.send(decision).is_ok()
    }

    /// The requests still waiting; one whose agent stopped waiting goes.
    pub fn list(&self) -> Vec<UseRequest> {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.retain(|(_, tx)| !tx.is_closed());
        pending.iter().map(|(r, _)| r.clone()).collect()
    }

    /// Drops a request the agent gave up on.
    pub fn forget(&self, id: &str) {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).retain(|(r, _)| r.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str) -> UseRequest {
        UseRequest {
            id: id.into(),
            agent: "Agent1".into(),
            entry_id: "ssn-alex".into(),
            entry_label: "Alex".into(),
            what: "Social Security number".into(),
            origin: "https://irs.gov".into(),
            field: "SSN".into(),
            offer_always: true,
            reason: "a new site".into(),
            requested_ms: 0,
        }
    }

    #[tokio::test]
    async fn an_answer_reaches_the_waiting_agent_once() {
        let u = Uses::default();
        let rx = u.ask(req("u1"));
        assert_eq!(u.list().len(), 1);
        assert!(u.answer("u1", Decision::Always));
        assert!(!u.answer("u1", Decision::Deny), "answered already");
        assert_eq!(rx.await.unwrap(), Decision::Always);
        assert!(u.list().is_empty());
    }

    #[test]
    fn a_request_nobody_waits_on_goes_and_cant_be_answered() {
        let u = Uses::default();
        drop(u.ask(req("u1")));
        assert!(u.list().is_empty());
        assert!(!u.answer("u1", Decision::Once));
    }

    #[test]
    fn decisions_are_parsed_by_name_only() {
        assert_eq!(Decision::parse("once"), Some(Decision::Once));
        assert_eq!(Decision::parse("always"), Some(Decision::Always));
        assert_eq!(Decision::parse("deny"), Some(Decision::Deny));
        assert_eq!(Decision::parse("ALWAYS"), None);
        assert_eq!(Decision::parse(""), None);
    }
}
