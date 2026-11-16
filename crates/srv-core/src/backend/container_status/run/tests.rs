// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use futures_util::stream;

use super::*;
use crate::backend::container_status::{Action, ContainerState, Published};

const ANN: &str = "6b6e9e6a-125d-47de-9594-0123456789ab";
const ORPHAN: &str = "0c1d2e3f-4a5b-4c6d-8e7f-a1b2c3d4e5f6";

type Item = Result<DockerEvent, String>;

/// A runtime that answers from scripts: the last answer of each script repeats.
#[derive(Default)]
struct Fake {
    pings: Mutex<VecDeque<Probe>>,
    lists: Mutex<VecDeque<Result<Vec<Listed>, String>>>,
    /// One per subscription. A stream with `open` stays quiet after its events; one without ends.
    streams: Mutex<VecDeque<(Vec<Item>, bool)>>,
    ping_calls: Mutex<usize>,
    subscriptions: Mutex<usize>,
}

fn next_of<T: Clone>(queue: &Mutex<VecDeque<T>>) -> Option<T> {
    let mut queue = queue.lock().unwrap();
    if queue.len() > 1 {
        queue.pop_front()
    } else {
        queue.front().cloned()
    }
}

impl Source for Arc<Fake> {
    async fn ping(&self) -> Probe {
        *self.ping_calls.lock().unwrap() += 1;
        next_of(&self.pings).unwrap_or(Probe { available: false, runtime: None, detail: None })
    }

    async fn list(&self) -> Result<Vec<Listed>, String> {
        next_of(&self.lists).unwrap_or(Ok(Vec::new()))
    }

    async fn events(&self) -> Result<BoxStream<'static, Item>, String> {
        *self.subscriptions.lock().unwrap() += 1;
        let (items, open) = next_of(&self.streams).unwrap_or((Vec::new(), true));
        let items = stream::iter(items);
        Ok(if open { items.chain(stream::pending()).boxed() } else { items.boxed() })
    }
}

fn up() -> Probe {
    Probe { available: true, runtime: Some("Docker".into()), detail: None }
}

fn down() -> Probe {
    Probe { available: false, runtime: None, detail: Some("no socket".into()) }
}

fn listed(id: &str, running: bool) -> Listed {
    Listed { name: format!("agentmux-{id}"), image: "agentmux-agent:1".into(), running, labelled: false }
}

fn event(action: Action, id: &str) -> DockerEvent {
    DockerEvent { action, name: format!("agentmux-{id}"), image: "agentmux-agent:1".into(), labelled: false }
}

fn watch(fake: &Arc<Fake>) -> (Arc<Hub>, Arc<Mutex<Vec<Published>>>, tokio::task::JoinHandle<()>) {
    let seen: Arc<Mutex<Vec<Published>>> = Default::default();
    let sink = Arc::clone(&seen);
    let hub = Arc::new(Hub::new(|| HashMap::from([(ANN.to_string(), "Ann".to_string())]), move |p| sink.lock().unwrap().push(p)));
    let task = tokio::spawn(run(Arc::clone(fake), Arc::clone(&hub), Timing::default()));
    (hub, seen, task)
}

fn container_states(seen: &Arc<Mutex<Vec<Published>>>) -> Vec<(&'static str, ContainerState)> {
    seen.lock()
        .unwrap()
        .iter()
        .filter_map(|p| match p {
            Published::Container(c) => Some((if c.agent_id.is_some() { "ann" } else { "orphan" }, c.state)),
            Published::Runtime(_) => None,
        })
        .collect()
}

fn runtime_states(seen: &Arc<Mutex<Vec<Published>>>) -> Vec<bool> {
    seen.lock().unwrap().iter().filter_map(|p| if let Published::Runtime(r) = p { Some(r.available) } else { None }).collect()
}

async fn pass(seconds: u64) {
    tokio::time::sleep(Duration::from_secs(seconds)).await;
}

#[tokio::test(start_paused = true)]
async fn the_list_then_each_event_is_published() {
    let fake = Arc::new(Fake::default());
    fake.pings.lock().unwrap().push_back(up());
    fake.lists.lock().unwrap().push_back(Ok(vec![listed(ANN, true)]));
    fake.streams.lock().unwrap().push_back((vec![Ok(event(Action::Die, ANN)), Ok(event(Action::Start, ORPHAN))], true));
    let (hub, seen, task) = watch(&fake);
    pass(1).await;
    assert_eq!(container_states(&seen), [("ann", ContainerState::Running), ("ann", ContainerState::Stopped), ("orphan", ContainerState::Running)]);
    assert_eq!(runtime_states(&seen), [true]);
    assert_eq!(hub.list().containers.len(), 2);
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn a_dropped_stream_is_subscribed_again_and_only_the_difference_is_published() {
    let fake = Arc::new(Fake::default());
    fake.pings.lock().unwrap().push_back(up());
    fake.lists.lock().unwrap().extend([Ok(vec![listed(ANN, true), listed(ORPHAN, true)]), Ok(vec![listed(ANN, true)])]);
    // The first stream ends at once; the second stays open.
    fake.streams.lock().unwrap().extend([(Vec::new(), false), (Vec::new(), true)]);
    let (_, seen, task) = watch(&fake);
    pass(1).await;
    assert_eq!(*fake.subscriptions.lock().unwrap(), 1);
    pass(Timing::default().retry.as_secs() + 1).await;
    assert_eq!(*fake.subscriptions.lock().unwrap(), 2, "subscribed again after the retry wait");
    assert_eq!(
        container_states(&seen),
        [("orphan", ContainerState::Running), ("ann", ContainerState::Running), ("orphan", ContainerState::Gone)],
        "the container that went while the stream was down is said once; the one that stayed is not said again"
    );
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn the_list_is_read_again_every_minute() {
    let fake = Arc::new(Fake::default());
    fake.pings.lock().unwrap().push_back(up());
    fake.lists.lock().unwrap().extend([Ok(vec![listed(ANN, true)]), Ok(vec![listed(ANN, false)])]);
    let (_, seen, task) = watch(&fake);
    pass(30).await;
    assert_eq!(container_states(&seen), [("ann", ContainerState::Running)]);
    pass(40).await;
    assert_eq!(container_states(&seen), [("ann", ContainerState::Running), ("ann", ContainerState::Stopped)]);
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn a_machine_with_no_runtime_says_so_once_and_then_leaves_it_alone() {
    let fake = Arc::new(Fake::default());
    fake.pings.lock().unwrap().push_back(down());
    let (hub, seen, task) = watch(&fake);
    pass(3_600).await;
    assert_eq!(runtime_states(&seen), [false]);
    assert_eq!(*fake.ping_calls.lock().unwrap(), 1, "nothing is looked at for an hour");
    // Something that needs a runtime wakes it: it looks again, and finds one.
    *fake.pings.lock().unwrap() = VecDeque::from([up()]);
    hub.wake();
    pass(1).await;
    assert_eq!(runtime_states(&seen), [false, true]);
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn a_runtime_that_stops_answering_is_said_and_watched_for() {
    let fake = Arc::new(Fake::default());
    fake.pings.lock().unwrap().extend([up(), down(), down(), up()]);
    let (_, seen, task) = watch(&fake);
    pass(35).await;
    assert_eq!(runtime_states(&seen), [true, false], "the 30-second ping found it gone");
    pass(70).await;
    assert_eq!(runtime_states(&seen), [true, false, true], "it is looked for again, not given up on");
    task.abort();
}
