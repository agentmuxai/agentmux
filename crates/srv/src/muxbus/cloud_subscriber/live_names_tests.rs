// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::{
    delivery_name, in_grace, is_held, live_agents, orphaned_subscriptions, seed, subscribe_missing, LiveAgent,
    NameDirectory, FORMER_SLUG_GRACE,
};
use crate::backend::reactive::handler::{LookupOutcome, ReactiveHandler};
use crate::muxbus::cloud_subscriber::{CloudSubscriber, CtrlMsg};

const LOAP_UID: &str = "a0dfc499-da34-4cb9-a510-936a70f75d4d";
const AGENTX_UID: &str = "5d1c2a0e-0000-4000-8000-000000000001";
const COUNTER: &str = "registration.no_uid.unspecified";

/// Loap-3's pane as it was on 2026-10-20: the pane registered `Loap-3`, then
/// the spawn rebound it under its chained slug, its `AGENTMUX_AGENT_ID`.
fn register_loap(h: &ReactiveHandler) {
    h.register_agent_full("Loap-3", "blk-loap", None, 0, None, Some(LOAP_UID), COUNTER).unwrap();
    let slug = "loap-loap-2-loap-3";
    h.register_agent_full(slug, "blk-loap", None, 7, Some(slug), Some(LOAP_UID), COUNTER).unwrap();
}

fn directory() -> NameDirectory {
    NameDirectory::from_defs([
        ("root-uid", "loap", "Loap"),
        ("fork2-uid", "loap-loap-2", "Loap #2"),
        (LOAP_UID, "loap-loap-2-loap-3", "Loap #3"),
        (AGENTX_UID, "agentx", "AgentX"),
        ("ghost-uid", "ghost", "Ghost"),
    ])
}

fn subscriber(names: &[&str]) -> (CloudSubscriber, tokio::sync::mpsc::UnboundedReceiver<CtrlMsg>) {
    let (ctrl_tx, ctrl_rx) = tokio::sync::mpsc::unbounded_channel();
    let set = names.iter().map(|n| n.to_string()).collect::<HashSet<_>>();
    (CloudSubscriber { ctrl_tx, agents: Arc::new(Mutex::new(set)) }, ctrl_rx)
}

fn added(rx: &mut tokio::sync::mpsc::UnboundedReceiver<CtrlMsg>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        match msg {
            CtrlMsg::AddAgent(n) => out.push(n),
            CtrlMsg::RemoveAgent(n) => panic!("unexpected removal of {n}"),
            CtrlMsg::ReloadToken => {}
        }
    }
    out
}

#[test]
fn a_rebound_fork_keeps_its_old_name_and_gains_its_slug() {
    let h = ReactiveHandler::new();
    register_loap(&h);
    assert!(!h.has_live_name("loap-3"), "the spawn's registration unbound the pane's own name");
    let live = live_agents(&h);
    assert_eq!(live, [LiveAgent { names: vec!["loap-loap-2-loap-3".into()], uid: Some(LOAP_UID.into()) }]);
    let dir = directory();
    let (sub, mut rx) = subscriber(&["loap-3"]);

    assert_eq!(subscribe_missing(&sub, &live, &dir), ["loap-loap-2-loap-3"]);
    assert!(orphaned_subscriptions(&sub.subscribed_agents(), &live, &dir).is_empty());
    assert!(!sub.drop_if_orphaned("loap-3", |a| h.has_live_name(a) || is_held(a, &live, &dir)));

    assert_eq!(sub.subscribed_agents(), ["loap-3", "loap-loap-2-loap-3"]);
    assert_eq!(added(&mut rx), ["loap-loap-2-loap-3"], "only the missing name is sent to the relay");
    assert!(subscribe_missing(&sub, &live, &dir).is_empty(), "nothing changed, nothing re-sent");
    assert!(added(&mut rx).is_empty());
}

#[test]
fn a_name_no_live_pane_answers_to_is_still_dropped() {
    let h = ReactiveHandler::new();
    register_loap(&h);
    let (live, dir) = (live_agents(&h), directory());
    let (sub, _rx) = subscriber(&["loap-3", "ghost", "nobody"]);

    // `ghost` has a definition, but no pane runs it; `nobody` has neither.
    let orphans = orphaned_subscriptions(&sub.subscribed_agents(), &live, &dir);
    assert_eq!(orphans, ["ghost", "nobody"]);
    for name in &orphans {
        assert!(sub.drop_if_orphaned(name, |a| h.has_live_name(a) || is_held(a, &live, &dir)));
    }
    assert_eq!(sub.subscribed_agents(), ["loap-3"]);
}

#[test]
fn a_dead_pane_holds_none_of_its_names() {
    let h = ReactiveHandler::new();
    register_loap(&h);
    h.set_block_liveness(Arc::new(|_| false));
    let live = live_agents(&h);
    assert!(live.is_empty());
    let subscribed = ["loap-3".to_string(), "loap-loap-2-loap-3".to_string()];
    assert_eq!(orphaned_subscriptions(&subscribed, &live, &directory()), subscribed);
}

/// A sign-in or reconnect subscribes, and the lease tick then provisions a
/// credential for, every name of every live pane: both of Loap-3's, AgentX's
/// display and stable names, and a pane with no definition by its own name.
#[test]
fn a_sign_in_subscribes_every_name_of_every_live_pane() {
    let h = ReactiveHandler::new();
    register_loap(&h);
    h.register_agent_full("AgentX", "blk-x", None, 0, Some("agentx"), Some(AGENTX_UID), COUNTER).unwrap();
    h.register_agent_full("Scratch", "blk-s", None, 0, None, None, COUNTER).unwrap();
    let (live, dir) = (live_agents(&h), directory());
    let agents = Mutex::new(HashSet::new());

    seed(&agents, &live, &dir);

    let mut got: Vec<String> = agents.lock().unwrap().iter().cloned().collect();
    got.sort();
    assert_eq!(got, ["agentx", "loap-3", "loap-loap-2-loap-3", "scratch"]);
    assert!(seed(&agents, &live, &dir).is_empty(), "a second seed adds nothing");
}

#[test]
fn a_message_for_the_old_name_is_delivered_to_the_pane_by_its_slug() {
    let h = ReactiveHandler::new();
    register_loap(&h);
    let (live, dir) = (live_agents(&h), directory());

    let target = delivery_name("Loap-3", &live, &dir);
    assert_eq!(target, "loap-loap-2-loap-3");
    assert!(matches!(h.lookup_by_name(&target), LookupOutcome::One(reg) if reg.block_id == "blk-loap"));
    assert_eq!(delivery_name("loap-loap-2-loap-3", &live, &dir), "loap-loap-2-loap-3");
    assert_eq!(delivery_name("ghost", &live, &dir), "ghost", "no live pane: unchanged, refused as before");
}

#[test]
fn a_slug_outranks_a_route_name_and_a_shared_name_names_no_one() {
    let dir = NameDirectory::from_defs([
        ("template", "claude", "Claude"),
        ("user", "claude-2", "Claude"),
        ("a", "twin-a", "Twin"),
        ("b", "twin-b", "Twin"),
        ("empty", "", ""),
    ]);
    assert_eq!(dir.uid_of("claude"), Some("template"));
    assert_eq!(dir.uid_of("Claude-2"), Some("user"));
    assert_eq!(dir.uid_of("twin"), None, "two agents answer to it");
    assert_eq!(dir.uid_of("twin-a"), Some("a"));
    assert_eq!(dir.uid_of("agent"), None, "an empty name gives no name");
}

/// Fork slug migration Phase 1: after Loap #3 is renamed to `loap-3`, its
/// former slug stays subscribed, receive-only, for the grace period after
/// the rename, and a message pulled for it goes to the pane under its
/// current slug. Once the period is over the directory leaves it out, and
/// the lease tick drops it as orphaned.
#[test]
fn a_former_slug_is_held_for_its_grace_period_then_dropped() {
    let now: i64 = 1_760_000_000_000;
    let day: i64 = 24 * 60 * 60 * 1000;
    let grace_days = (FORMER_SLUG_GRACE.as_millis() as i64) / day;
    assert_eq!(grace_days, 30);
    assert!(in_grace(now - 29 * day, now));
    assert!(!in_grace(now - 31 * day, now));
    assert!(!in_grace(0, now), "an unknown retirement time is not held");

    let h = ReactiveHandler::new();
    h.register_agent_full("Loap-3", "blk-loap", None, 0, Some("loap-3"), Some(LOAP_UID), COUNTER).unwrap();
    let live = live_agents(&h);
    let defs = [(LOAP_UID, "loap-3", "Loap #3"), (AGENTX_UID, "agentx", "AgentX")];
    let former = [(LOAP_UID, "loap-loap-2-loap-3"), (LOAP_UID, "agentx")];
    let within = NameDirectory::from_defs_and_former(defs, former);
    let (sub, mut rx) = subscriber(&["loap-3"]);

    assert_eq!(subscribe_missing(&sub, &live, &within), ["loap-loap-2-loap-3"]);
    assert_eq!(added(&mut rx), ["loap-loap-2-loap-3"]);
    assert!(orphaned_subscriptions(&sub.subscribed_agents(), &live, &within).is_empty());
    assert_eq!(delivery_name("loap-loap-2-loap-3", &live, &within), "loap-3");
    assert!(!is_held("agentx", &live, &within), "another agent's slug is never held as a former one");

    let after = NameDirectory::from_defs(defs);
    assert_eq!(orphaned_subscriptions(&sub.subscribed_agents(), &live, &after), ["loap-loap-2-loap-3"]);
    assert!(sub.drop_if_orphaned("loap-loap-2-loap-3", |a| h.has_live_name(a) || is_held(a, &live, &after)));
    assert_eq!(sub.subscribed_agents(), ["loap-3"]);
}
