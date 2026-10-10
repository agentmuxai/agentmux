// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agents' requests to install a widget, waiting for the user
//! (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §10).
//!
//! `WidgetInstall` copies the package in (it then needs approval like any
//! other) and asks here. The UI shows each request as a prompt; the user's
//! answer reaches srv only through the host's approval route, which answers
//! every request for that version. An agent can ask; it can never answer.

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use super::widget_packages::{WidgetKind, WidgetPackageInfo};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetInstallRequest {
    pub id: String,
    pub hash: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub kind: WidgetKind,
    pub permissions: Vec<String>,
    /// The agent that asked.
    pub agent: String,
    #[ts(type = "number")]
    pub requested_ms: u64,
}

impl WidgetInstallRequest {
    pub fn of(pkg: &WidgetPackageInfo, agent: &str) -> Self {
        Self {
            id: pkg.id.clone(),
            hash: pkg.hash.clone(),
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            description: pkg.description.clone(),
            kind: pkg.kind.clone(),
            permissions: pkg.permissions.clone(),
            agent: agent.to_string(),
            requested_ms: agentmux_common::time::now_ms_u64(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Approved,
    Declined,
}

#[derive(Default)]
pub struct Requests {
    pending: Mutex<Vec<(WidgetInstallRequest, Vec<oneshot::Sender<Answer>>)>>,
}

pub fn requests() -> &'static Requests {
    static REQUESTS: OnceLock<Requests> = OnceLock::new();
    REQUESTS.get_or_init(Requests::default)
}

/// At most this many requests wait at once; the oldest goes first.
const MAX_PENDING: usize = 32;

impl Requests {
    /// Asks the user; the receiver gets their answer. Asking again for the
    /// same version joins the request already waiting, and replaces any
    /// request for an older version of the same widget.
    pub fn ask(&self, req: WidgetInstallRequest) -> oneshot::Receiver<Answer> {
        let (tx, rx) = oneshot::channel();
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.retain(|(r, _)| r.id != req.id || r.hash == req.hash);
        if let Some((_, waiters)) = pending.iter_mut().find(|(r, _)| r.id == req.id && r.hash == req.hash) {
            waiters.push(tx);
        } else {
            if pending.len() >= MAX_PENDING {
                pending.remove(0);
            }
            pending.push((req, vec![tx]));
        }
        rx
    }

    /// The user's answer for `id` at `hash`; returns whether anything waited.
    /// Any answer about a widget also ends requests for its other versions,
    /// which the user has now seen past.
    pub fn answer(&self, id: &str, hash: &str, answer: Answer) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let mut found = false;
        pending.retain_mut(|(r, waiters)| {
            if r.id != id {
                return true;
            }
            let this_version = r.hash == hash;
            found |= this_version;
            for tx in waiters.drain(..) {
                let _ = tx.send(if this_version { answer } else { Answer::Declined });
            }
            false
        });
        found
    }

    /// Declines every request for `id`, whatever version (its package was
    /// uninstalled); returns whether any waited.
    pub fn decline_all(&self, id: &str) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let before = pending.len();
        pending.retain_mut(|(r, waiters)| {
            if r.id != id {
                return true;
            }
            for tx in waiters.drain(..) {
                let _ = tx.send(Answer::Declined);
            }
            false
        });
        pending.len() != before
    }

    pub fn list(&self) -> Vec<WidgetInstallRequest> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).iter().map(|(r, _)| r.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str, hash: &str, agent: &str) -> WidgetInstallRequest {
        WidgetInstallRequest {
            id: id.into(),
            hash: hash.into(),
            name: id.into(),
            version: "1.0.0".into(),
            description: None,
            kind: WidgetKind::Sandboxed,
            permissions: vec![],
            agent: agent.into(),
            requested_ms: 0,
        }
    }

    #[tokio::test]
    async fn the_users_answer_reaches_every_agent_that_asked_for_that_version() {
        let r = Requests::default();
        let a = r.ask(req("acme.x", "h1", "Lark"));
        let b = r.ask(req("acme.x", "h1", "Korp"));
        assert_eq!(r.list().len(), 1, "one prompt per version");
        assert!(r.answer("acme.x", "h1", Answer::Approved));
        assert_eq!(a.await.unwrap(), Answer::Approved);
        assert_eq!(b.await.unwrap(), Answer::Approved);
        assert!(r.list().is_empty());
        assert!(!r.answer("acme.x", "h1", Answer::Approved), "nothing left to answer");
    }

    #[tokio::test]
    async fn uninstalling_declines_the_widgets_requests() {
        let r = Requests::default();
        let waiting = r.ask(req("acme.x", "h1", "Lark"));
        let other = r.ask(req("acme.y", "h1", "Lark"));
        assert!(r.decline_all("acme.x"));
        assert_eq!(waiting.await.unwrap(), Answer::Declined);
        assert_eq!(r.list().iter().map(|q| q.id.as_str()).collect::<Vec<_>>(), ["acme.y"]);
        assert!(!r.decline_all("acme.x"), "nothing left for it");
        drop(other);
    }

    #[tokio::test]
    async fn a_newer_version_replaces_the_older_request() {
        let r = Requests::default();
        let old = r.ask(req("acme.x", "h1", "Lark"));
        let new = r.ask(req("acme.x", "h2", "Lark"));
        assert!(old.await.is_err(), "the old request is dropped");
        assert_eq!(r.list().iter().map(|q| q.hash.as_str()).collect::<Vec<_>>(), ["h2"]);
        let other = r.ask(req("acme.y", "h9", "Lark"));
        r.answer("acme.x", "h2", Answer::Declined);
        assert_eq!(new.await.unwrap(), Answer::Declined);
        assert_eq!(r.list().len(), 1, "another widget's request stays");
        drop(other);
    }
}
