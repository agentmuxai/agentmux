// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The names the relay subscription holds: every name a live pane answers to.
//!
//! A pane is bound in the reactive registry by a display and a stable name,
//! but its agent also answers to its definition's slug and its display name's
//! route name, and a re-registration can leave one of those unbound while the
//! pane keeps running: a fork whose stored slug is chained (`loap-loap-2-loap-3`)
//! is registered as `Loap-3` by the pane, then rebound by its spawn under the
//! slug, which no path subscribed. Its subscription `loap-3` then looked
//! orphaned and was dropped, and the slug was never subscribed, so nothing
//! from the relay reached it. Here a subscription is matched against every
//! name of a live agent before it is dropped, every such name is subscribed,
//! and a message pulled for one is delivered to the pane that runs its agent.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::backend::reactive::handler::ReactiveHandler;

use super::CloudSubscriber;

/// One live registration: its lowercased names, in the registry's order
/// (display first), and its agent's UID when the registry has one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct LiveAgent {
    pub(super) names: Vec<String>,
    pub(super) uid: Option<String>,
}

/// Lowercased name → the UID of the one agent definition that answers to it.
#[derive(Debug, Default)]
pub(super) struct NameDirectory {
    by_name: HashMap<String, String>,
}

impl NameDirectory {
    /// From `(uid, slug, display name)` rows. A slug outranks a display
    /// name's route name, as `Store::agent_defs_for_target` resolves a
    /// target, and a name several definitions answer to at its best rank
    /// names none of them: an agent is never kept under a name that isn't
    /// only its own. Former names are left out on purpose: holding one would
    /// renew its WAN lease and keep it from an agent that takes it later.
    pub(super) fn from_defs<'a>(defs: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>) -> Self {
        let mut ranked: HashMap<String, (u8, HashSet<&'a str>)> = HashMap::new();
        for (uid, slug, name) in defs {
            if uid.is_empty() {
                continue;
            }
            let slug = Some(slug.trim().to_string()).filter(|s| agentmux_common::slug::is_agent_id(s));
            let route = crate::backend::reactive::sanitize::registrable_id(name);
            for (rank, key) in [(0u8, slug), (1, route)] {
                let Some(key) = key.map(|k| k.to_lowercase()) else { continue };
                let entry = ranked.entry(key).or_insert((rank, HashSet::new()));
                if rank < entry.0 {
                    *entry = (rank, HashSet::new());
                }
                if rank == entry.0 {
                    entry.1.insert(uid);
                }
            }
        }
        let by_name = ranked
            .into_iter()
            .filter_map(|(name, (_, uids))| {
                let mut uids = uids.into_iter();
                match (uids.next(), uids.next()) {
                    (Some(uid), None) => Some((name, uid.to_string())),
                    _ => None,
                }
            })
            .collect();
        Self { by_name }
    }

    fn uid_of(&self, name: &str) -> Option<&str> {
        self.by_name.get(&name.to_lowercase()).map(String::as_str)
    }
}

/// Every name the subscription should hold for `live`: each registration's
/// own names and every name the directory gives its agent. Lowercased, sorted.
pub(super) fn wanted(live: &[LiveAgent], dir: &NameDirectory) -> Vec<String> {
    let uids: HashSet<&str> = live.iter().filter_map(|a| a.uid.as_deref()).collect();
    let mut names: Vec<String> = live.iter().flat_map(|a| a.names.iter().cloned()).collect();
    names.extend(
        dir.by_name
            .iter()
            .filter(|(_, uid)| uids.contains(uid.as_str()))
            .map(|(name, _)| name.clone()),
    );
    names.sort();
    names.dedup();
    names
}

/// The live registration `name` belongs to: one bound by it, else the one
/// running the agent the directory says it names.
fn holder<'a>(name: &str, live: &'a [LiveAgent], dir: &NameDirectory) -> Option<&'a LiveAgent> {
    let key = name.to_lowercase();
    live.iter().find(|a| a.names.contains(&key)).or_else(|| {
        let uid = dir.uid_of(&key)?;
        live.iter().find(|a| a.uid.as_deref() == Some(uid))
    })
}

/// Does a live pane answer to `name`, by binding or through its agent?
pub(super) fn is_held(name: &str, live: &[LiveAgent], dir: &NameDirectory) -> bool {
    holder(name, live, dir).is_some()
}

/// Subscribed names no live pane answers to. Every way into the subscription
/// runs with a live registration, so one without is a leak from a teardown
/// that missed it; renewing its WAN lease would fence the agent everywhere
/// else for as long as this srv runs.
pub(super) fn orphaned_subscriptions(subscribed: &[String], live: &[LiveAgent], dir: &NameDirectory) -> Vec<String> {
    subscribed.iter().filter(|a| !is_held(a, live, dir)).cloned().collect()
}

/// The name to deliver a message pulled for `name` to: `name` when a live
/// pane is bound by it (or none answers to it), else the holder's name that
/// the directory gives the same agent (its slug), which the registry resolves
/// to that pane and the transcript-request rules resolve to its definition.
pub(super) fn delivery_name(name: &str, live: &[LiveAgent], dir: &NameDirectory) -> String {
    let key = name.to_lowercase();
    match holder(name, live, dir) {
        Some(agent) if !agent.names.contains(&key) => agent
            .names
            .iter()
            .find(|n| agent.uid.is_some() && dir.uid_of(n) == agent.uid.as_deref())
            .or_else(|| agent.names.first())
            .cloned()
            .unwrap_or_else(|| name.to_string()),
        _ => name.to_string(),
    }
}

/// Subscribe each wanted name `sub` doesn't hold yet (`add_agent`, which
/// tells the WS loop). Returns the names added; a second call with nothing
/// changed adds none.
pub(super) fn subscribe_missing(sub: &CloudSubscriber, live: &[LiveAgent], dir: &NameDirectory) -> Vec<String> {
    let held: HashSet<String> = sub.agents.lock().unwrap().clone();
    let added: Vec<String> = wanted(live, dir).into_iter().filter(|n| !held.contains(n)).collect();
    for name in &added {
        sub.add_agent(name);
    }
    log_added(&added);
    added
}

/// Put every wanted name in `agents` directly, before a session's first
/// `subscribe` (which carries the whole set).
pub(super) fn seed(agents: &Mutex<HashSet<String>>, live: &[LiveAgent], dir: &NameDirectory) -> Vec<String> {
    let mut agents = agents.lock().unwrap();
    let added: Vec<String> = wanted(live, dir).into_iter().filter(|n| agents.insert(n.clone())).collect();
    drop(agents);
    log_added(&added);
    added
}

fn log_added(added: &[String]) {
    if !added.is_empty() {
        tracing::info!(names = ?added, "cloud_subscriber: subscribed names a live pane answers to");
    }
}

/// The registry's live registrations now, by the same liveness the lease
/// tick has always used (`has_live_name`). Takes the handler's lock once per
/// call, never while holding the subscriber's.
pub(super) fn live_agents(handler: &ReactiveHandler) -> Vec<LiveAgent> {
    handler
        .list_agents()
        .into_iter()
        .filter_map(|reg| {
            let names: Vec<String> = handler
                .names_for_block(&reg.block_id)
                .iter()
                .map(|n| n.to_lowercase())
                .collect();
            names
                .iter()
                .any(|n| handler.has_live_name(n))
                .then_some(LiveAgent { names, uid: reg.uid })
        })
        .collect()
}

/// How long a loaded directory is reused. Names change only on a rename or a
/// new agent; the lease tick and every registration consult it.
const DIRECTORY_TTL: Duration = Duration::from_secs(60);

static DIRECTORY: Mutex<Option<(Instant, Arc<NameDirectory>)>> = Mutex::new(None);

/// The definitions' names, from this channel's object store, cached for
/// [`DIRECTORY_TTL`]. A failed read reuses the last directory, uncached, so a
/// store hiccup doesn't drop every alias at once.
pub(super) async fn directory() -> Arc<NameDirectory> {
    let cached = DIRECTORY.lock().unwrap().clone();
    if let Some((at, dir)) = &cached {
        if at.elapsed() < DIRECTORY_TTL {
            return Arc::clone(dir);
        }
    }
    match tokio::task::spawn_blocking(load_directory).await {
        Ok(Some(dir)) => {
            let dir = Arc::new(dir);
            *DIRECTORY.lock().unwrap() = Some((Instant::now(), Arc::clone(&dir)));
            dir
        }
        _ => cached.map(|(_, dir)| dir).unwrap_or_default(),
    }
}

fn load_directory() -> Option<NameDirectory> {
    let store = crate::backend::identity_spawn::row_store()?;
    match store.agent_def_list() {
        Ok(defs) => Some(NameDirectory::from_defs(
            defs.iter().map(|d| (d.id.as_str(), d.slug.as_str(), d.name.as_str())),
        )),
        Err(e) => {
            tracing::warn!(error = %e, "cloud_subscriber: agent names not read; keeping the last ones");
            None
        }
    }
}

/// Wakes a session to subscribe a pane that just registered
/// ([`registration_changed`]).
static REGISTERED: std::sync::LazyLock<tokio::sync::Notify> = std::sync::LazyLock::new(tokio::sync::Notify::new);

/// A pane registered: a session subscribes its names now rather than at the
/// next lease tick. No locks taken, so safe under the handler's.
pub(crate) fn registration_changed() {
    REGISTERED.notify_one();
}

pub(super) async fn registrations_changed() {
    REGISTERED.notified().await;
}

/// [`subscribe_missing`] against the global subscriber, the registry and
/// the directory, for a session's registration wake and its lease tick.
/// Returns the snapshot it used, for the tick's orphan check.
pub(super) async fn subscribe_live(handler: &ReactiveHandler) -> (Vec<LiveAgent>, Arc<NameDirectory>) {
    let live = live_agents(handler);
    let dir = directory().await;
    if let Some(sub) = super::get_global_subscriber() {
        subscribe_missing(sub, &live, &dir);
    }
    (live, dir)
}

/// [`delivery_name`] for a message pulled for `name`; reads the store only
/// when no live pane is bound by `name`.
pub(super) async fn delivery_target(handler: &ReactiveHandler, name: &str) -> String {
    if handler.has_live_name(name) {
        return name.to_string();
    }
    let live = live_agents(handler);
    delivery_name(name, &live, &*directory().await)
}

#[cfg(test)]
#[path = "live_names_tests.rs"]
mod tests;
