// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, Mutex};

use super::*;

const ANN: &str = "6b6e9e6a-125d-47de-9594-0123456789ab";
const ORPHAN: &str = "0c1d2e3f-4a5b-4c6d-8e7f-a1b2c3d4e5f6";

fn name_of(id: &str) -> String {
    format!("agentmux-{id}")
}

fn hub() -> (Hub, Arc<Mutex<Vec<Published>>>) {
    let seen: Arc<Mutex<Vec<Published>>> = Default::default();
    let sink = Arc::clone(&seen);
    let hub = Hub::new(|| HashMap::from([(ANN.to_string(), "Ann".to_string())]), move |p| sink.lock().unwrap().push(p));
    (hub, seen)
}

fn event(action: Action, id: &str) -> DockerEvent {
    DockerEvent { action, name: name_of(id), image: "agentmux-agent:1".into(), labelled: false }
}

fn listed(id: &str, running: bool) -> Listed {
    Listed { name: name_of(id), image: "agentmux-agent:1".into(), running, labelled: false }
}

fn states(seen: &Arc<Mutex<Vec<Published>>>) -> Vec<(String, ContainerState)> {
    seen.lock()
        .unwrap()
        .iter()
        .filter_map(|p| match p {
            Published::Container(c) => Some((c.name.clone(), c.state)),
            Published::Runtime(_) => None,
        })
        .collect()
}

#[test]
fn each_docker_event_moves_one_container_and_publishes_once() {
    let (hub, seen) = hub();
    for action in [Action::Create, Action::Start, Action::Die, Action::Stop, Action::Start, Action::Destroy] {
        hub.on_event(event(action, ANN));
    }
    // `die` then `stop` is one move to stopped, and the repeat says nothing.
    let want = [ContainerState::Starting, ContainerState::Running, ContainerState::Stopped, ContainerState::Running, ContainerState::Gone];
    assert_eq!(states(&seen), want.map(|s| (name_of(ANN), s)));
    assert!(hub.list().containers.is_empty(), "a removed container is only an event");
    // A removed container is said once.
    hub.on_event(event(Action::Destroy, ANN));
    assert_eq!(states(&seen).len(), 5);
}

#[test]
fn a_state_keeps_the_time_it_was_entered() {
    let mut registry = Registry::default();
    let agents = HashMap::from([(ANN.to_string(), "Ann".to_string())]);
    registry.apply(event(Action::Start, ANN), 100);
    registry.apply(event(Action::Start, ANN), 900);
    assert_eq!(registry.changes(&agents, 1_000)[0].since_ms, 100);
    registry.apply(event(Action::Die, ANN), 2_000);
    assert_eq!(registry.changes(&agents, 2_000)[0].since_ms, 2_000);
}

#[test]
fn an_orphan_is_listed_without_an_agent_and_another_container_is_not() {
    let (hub, _) = hub();
    hub.on_listed(vec![
        listed(ANN, true),
        listed(ORPHAN, false),
        // Named under the prefix, but no agent's: not an agent id, no label, no definition.
        listed("cli-cache", true),
        Listed { name: "postgres".into(), image: "postgres:16".into(), running: true, labelled: false },
    ]);
    let list = hub.list();
    let got: Vec<(&str, Option<&str>, Option<&str>, ContainerState)> =
        list.containers.iter().map(|c| (c.name.as_str(), c.agent_id.as_deref(), c.agent_name.as_deref(), c.state)).collect();
    assert_eq!(
        got,
        [
            (name_of(ORPHAN).as_str(), None, None, ContainerState::Stopped),
            (name_of(ANN).as_str(), Some(ANN), Some("Ann"), ContainerState::Running),
        ]
    );
    // Null, present: not left out.
    let json = serde_json::to_value(&list.containers[0]).unwrap();
    assert!(json["agent_id"].is_null() && json.get("agent_id").is_some() && json["agent_name"].is_null(), "{json}");
    assert_eq!(json["image"], "agentmux-agent:1");
    assert_eq!(json["state"], "stopped");
}

#[test]
fn a_labelled_container_is_listed_whatever_its_id_looks_like() {
    let (hub, _) = hub();
    hub.on_listed(vec![Listed { name: "agentmux-ann".into(), image: String::new(), running: true, labelled: true }]);
    assert_eq!(hub.list().containers.len(), 1);
}

#[test]
fn a_reconcile_publishes_only_the_difference() {
    let (hub, seen) = hub();
    hub.on_listed(vec![listed(ANN, true), listed(ORPHAN, true)]);
    assert_eq!(states(&seen).len(), 2);
    // The same list again: nothing.
    hub.on_listed(vec![listed(ANN, true), listed(ORPHAN, true)]);
    assert_eq!(states(&seen).len(), 2);
    // One stopped while the stream was down, one removed.
    hub.on_listed(vec![listed(ANN, false)]);
    assert_eq!(
        states(&seen)[2..],
        [(name_of(ANN), ContainerState::Stopped), (name_of(ORPHAN), ContainerState::Gone)]
    );
}

#[test]
fn srv_starting_a_container_shows_before_docker_says_anything() {
    let (hub, seen) = hub();
    hub.on_starting(&name_of(ANN), "agentmux-agent:1");
    hub.on_event(event(Action::Start, ANN));
    // A container already running is not shown as starting again.
    hub.on_starting(&name_of(ANN), "");
    assert_eq!(states(&seen), [(name_of(ANN), ContainerState::Starting), (name_of(ANN), ContainerState::Running)]);
    // A restart keeps the image it had.
    hub.on_event(event(Action::Stop, ANN));
    hub.on_starting(&name_of(ANN), "");
    assert_eq!(hub.list().containers[0].image, "agentmux-agent:1");
}

#[test]
fn the_runtime_is_said_when_it_changes_and_not_when_it_is_looked_at_again() {
    let (hub, seen) = hub();
    let down = || Probe { available: false, runtime: None, detail: Some("no socket".into()) };
    hub.on_runtime(down());
    hub.on_runtime(down());
    let up = Probe { available: true, runtime: Some("Docker".into()), detail: None };
    hub.on_runtime(up.clone());
    hub.on_runtime(up);
    let said: Vec<bool> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|p| if let Published::Runtime(r) = p { Some(r.available) } else { None })
        .collect();
    assert_eq!(said, [false, true]);
    let json = serde_json::to_value(hub.list().runtime).unwrap();
    assert_eq!((json["available"].clone(), json["runtime"].clone()), (true.into(), "Docker".into()));
    assert!(json.get("detail").is_none(), "{json}");
}

#[test]
fn a_machine_with_no_runtime_and_no_container_is_dormant_until_it_has_one() {
    let (hub, _) = hub();
    assert!(!hub.is_dormant(), "not before the first look");
    hub.on_runtime(Probe { available: false, runtime: None, detail: None });
    assert!(hub.is_dormant());
    hub.on_event(event(Action::Create, ANN));
    assert!(!hub.is_dormant(), "a container is known");
    hub.on_event(event(Action::Destroy, ANN));
    assert!(hub.is_dormant());
    hub.on_runtime(Probe { available: true, runtime: None, detail: None });
    hub.on_runtime(Probe { available: false, runtime: None, detail: None });
    assert!(!hub.is_dormant(), "a runtime that was there and stopped answering is watched");
}

#[test]
fn an_event_for_another_container_is_ignored() {
    let (hub, seen) = hub();
    hub.on_event(DockerEvent { action: Action::Start, name: "postgres".into(), image: "postgres:16".into(), labelled: false });
    assert!(states(&seen).is_empty());
}

#[test]
fn a_container_srv_is_bringing_up_survives_a_list_that_does_not_name_it_for_a_while() {
    let mut registry = Registry::default();
    let agents = HashMap::from([(ANN.to_string(), "Ann".to_string())]);
    registry.starting(&name_of(ANN), "agentmux-agent:1", 1_000);
    // Docker has not made it yet (the image is downloading): not gone, and nothing new to say.
    registry.replace(Vec::new(), 61_000);
    let first = registry.changes(&agents, 61_000);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].state, ContainerState::Starting);
    registry.replace(Vec::new(), 121_000);
    assert!(registry.changes(&agents, 121_000).is_empty(), "the same state says nothing again");
    // Made and not started: still starting, with the time it started.
    registry.replace(vec![listed(ANN, false)], 181_000);
    assert!(registry.changes(&agents, 181_000).is_empty());
    assert_eq!(registry.containers()[0].since_ms, 1_000);
    // A start that never came is not kept for good.
    registry.replace(vec![listed(ANN, false)], 1_000 + STARTING_GRACE_MS);
    assert_eq!(registry.changes(&agents, 1_000 + STARTING_GRACE_MS)[0].state, ContainerState::Stopped);
    registry.starting(&name_of(ORPHAN), "", 2_000_000);
    registry.replace(Vec::new(), 2_000_000 + STARTING_GRACE_MS);
    let gone = registry.changes(&agents, 2_000_000 + STARTING_GRACE_MS);
    assert!(gone.iter().any(|c| c.state == ContainerState::Gone), "{gone:?}");
}
