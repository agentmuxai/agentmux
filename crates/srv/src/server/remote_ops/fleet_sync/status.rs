// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the fleet Global Memory sync did, per peer, for the Memory list to show: when each peer was
//! last synced, how many entries went, why it could not be (a refused list, an unreachable machine),
//! and which entries the peer answered but did not apply (an owner's `machine` entry, say). The
//! writer reads this instead of finding out from the peer's stewards that an entry never arrived.
//!
//! It also keeps the entries this machine replaced because a newer write came from the fleet while
//! its own `machine` row was only a default (`backend::memory::fleet`), so a steward can see what
//! changed on its own and revert it from the entry's history. In memory only: it starts empty with
//! the process; the history of each entry is what lasts.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

/// The most replaced entries kept; the oldest go first.
const MAX_ADOPTED: usize = 50;

#[derive(Default)]
struct PeerState {
    synced_at_ms: Option<i64>,
    sent: u64,
    error: Option<String>,
    /// Entry id -> why the peer did not apply it, from the last pass.
    not_applied: BTreeMap<String, String>,
}

struct Adopted {
    id: String,
    name: String,
    from: String,
    at_ms: i64,
}

#[derive(Default)]
struct Inner {
    peers: BTreeMap<String, PeerState>,
    adopted: VecDeque<Adopted>,
}

#[derive(Default)]
pub(crate) struct Status {
    inner: Mutex<Inner>,
}

/// The one this process keeps.
pub(crate) fn global() -> &'static Status {
    static STATUS: OnceLock<Status> = OnceLock::new();
    STATUS.get_or_init(Status::default)
}

impl Status {
    /// A pass over `host` finished. Returns the entries newly not applied (not already so on the
    /// last pass), for the log.
    pub(crate) fn synced(&self, host: &str, at_ms: i64, sent: usize, not_applied: BTreeMap<String, String>) -> Vec<(String, String)> {
        let mut inner = self.inner.lock().unwrap();
        let peer = inner.peers.entry(host.to_string()).or_default();
        let fresh = not_applied.iter().filter(|(id, why)| peer.not_applied.get(*id) != Some(*why)).map(|(i, w)| (i.clone(), w.clone())).collect();
        peer.synced_at_ms = Some(at_ms);
        peer.sent += sent as u64;
        peer.error = None;
        peer.not_applied = not_applied;
        fresh
    }

    /// A pass over `host` could not be made.
    pub(crate) fn failed(&self, host: &str, why: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.peers.entry(host.to_string()).or_default().error = Some(why.to_string());
    }

    /// Forget peers that are no longer on the network.
    pub(crate) fn keep_only(&self, hosts: &[&str]) {
        self.inner.lock().unwrap().peers.retain(|host, _| hosts.contains(&host.as_str()));
    }

    /// A newer write from `from` replaced entry `id` here, which was `machine` only by default.
    pub(crate) fn adopted(&self, id: &str, name: &str, from: &str, at_ms: i64) {
        let mut inner = self.inner.lock().unwrap();
        inner.adopted.push_back(Adopted { id: id.to_string(), name: name.to_string(), from: from.to_string(), at_ms });
        while inner.adopted.len() > MAX_ADOPTED {
            inner.adopted.pop_front();
        }
    }

    /// The Memory list's `sync` field.
    pub(crate) fn snapshot(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        let peers: serde_json::Map<String, Value> = inner
            .peers
            .iter()
            .map(|(host, p)| {
                let state = json!({
                    "synced_at": p.synced_at_ms,
                    "sent": p.sent,
                    "error": p.error,
                    "not_applied": p.not_applied,
                });
                (host.clone(), state)
            })
            .collect();
        let adopted: Vec<Value> =
            inner.adopted.iter().rev().map(|a| json!({ "id": a.id, "name": a.name, "from": a.from, "at": a.at_ms })).collect();
        json!({ "peers": peers, "replaced_here": adopted })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skipped(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(i, w)| (i.to_string(), w.to_string())).collect()
    }

    #[test]
    fn a_pass_reports_what_the_peer_did_not_apply_and_names_only_the_new_ones_for_the_log() {
        let status = Status::default();
        let first = status.synced("charlie", 1_000, 2, skipped(&[("a", "machine_scope")]));
        assert_eq!(first, [("a".to_string(), "machine_scope".to_string())]);
        let again = status.synced("charlie", 2_000, 0, skipped(&[("a", "machine_scope"), ("b", "protected")]));
        assert_eq!(again, [("b".to_string(), "protected".to_string())], "a is not news");

        let snap = status.snapshot();
        assert_eq!(snap["peers"]["charlie"]["sent"], 2);
        assert_eq!(snap["peers"]["charlie"]["synced_at"], 2_000);
        assert_eq!(snap["peers"]["charlie"]["not_applied"]["b"], "protected");

        status.synced("charlie", 3_000, 1, BTreeMap::new());
        assert_eq!(status.snapshot()["peers"]["charlie"]["not_applied"], json!({}), "it applied after all");
    }

    #[test]
    fn a_peer_that_cannot_be_reached_shows_why_and_one_that_left_is_forgotten() {
        let status = Status::default();
        status.failed("old-build", "list: peer_cannot_do_remote_ops");
        let snap = status.snapshot();
        assert_eq!(snap["peers"]["old-build"]["error"], "list: peer_cannot_do_remote_ops");
        assert_eq!(snap["peers"]["old-build"]["synced_at"], Value::Null);

        status.synced("charlie", 1, 0, BTreeMap::new());
        status.keep_only(&["charlie"]);
        assert!(status.snapshot()["peers"].get("old-build").is_none());
        assert!(status.snapshot()["peers"].get("charlie").is_some());
    }

    #[test]
    fn replaced_entries_are_listed_newest_first_and_the_oldest_drop_off() {
        let status = Status::default();
        for n in 0..(MAX_ADOPTED + 3) {
            status.adopted(&format!("e{n}"), "Rules", "loap@narko", n as i64);
        }
        let snap = status.snapshot();
        let list = snap["replaced_here"].as_array().unwrap();
        assert_eq!(list.len(), MAX_ADOPTED);
        assert_eq!(list[0]["id"], format!("e{}", MAX_ADOPTED + 2));
        assert_eq!(list[MAX_ADOPTED - 1]["id"], "e3");
    }
}
