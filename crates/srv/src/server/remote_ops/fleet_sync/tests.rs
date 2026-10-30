// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Mutex;

use futures_util::future::BoxFuture;

use super::*;
use crate::backend::fleet_memory::{apply_put, apply_remove, Actor, FleetEntry, FleetRemoval};

const WRITER: Actor<'static> = Actor { label: "loap@narko", uid: "" };

fn machine() -> Arc<Store> {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.bundle_scope_ensure().unwrap();
    store
}

fn write(store: &Store, id: &str, text: &str, updated_at: i64) {
    let entry = FleetEntry {
        id: id.into(),
        name: format!("Rules {id}"),
        description: String::new(),
        instructions: text.into(),
        instructions_by_provider: "{}".into(),
        context_files: "[]".into(),
        updated_at,
        edited_by: "armory-ui".into(),
    };
    apply_put(store, &entry, WRITER).unwrap();
}

/// Another machine: answers the three operations from its own store, and remembers what it was asked.
struct Fake {
    store: Arc<Store>,
    asked: Arc<Mutex<Vec<String>>>,
    /// When set, `memory.global.list` is refused with this reason.
    refuse_list: Arc<Mutex<Option<&'static str>>>,
}

impl Fake {
    fn new(store: Arc<Store>) -> Self {
        Self { store, asked: Default::default(), refuse_list: Default::default() }
    }

    fn call(&self) -> impl Fn(String, &'static str, Value) -> BoxFuture<'static, RemoteOpResult> {
        let (store, asked, refuse) = (self.store.clone(), self.asked.clone(), self.refuse_list.clone());
        move |_host, op, params| {
            let (store, asked, refuse) = (store.clone(), asked.clone(), refuse.clone());
            Box::pin(async move {
                asked.lock().unwrap().push(op.to_string());
                let actor = Actor { label: "synced@narko", uid: "" };
                match op {
                    "memory.global.list" => {
                        if let Some(reason) = *refuse.lock().unwrap() {
                            return RemoteOpResult::refused(reason);
                        }
                        RemoteOpResult::done(json!({ "entries": fleet_memory::list_meta(&store).unwrap() }))
                    }
                    "memory.global.put" => {
                        let entry: FleetEntry = serde_json::from_value(params).unwrap();
                        RemoteOpResult::done(apply_put(&store, &entry, actor).unwrap().to_json())
                    }
                    "memory.global.remove" => {
                        let removal: FleetRemoval = serde_json::from_value(params).unwrap();
                        RemoteOpResult::done(apply_remove(&store, &removal, actor).unwrap().to_json())
                    }
                    other => RemoteOpResult::refused(&format!("no such op {other}")),
                }
            })
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

fn peer(name: &str) -> Peer {
    Peer { key: format!("id-{name}"), host: name.into() }
}

fn ids(store: &Store) -> Vec<(String, bool)> {
    let mut v: Vec<_> = fleet_memory::list_meta(store).unwrap().into_iter().map(|m| (m.id, m.removed)).collect();
    v.sort();
    v
}

#[tokio::test]
async fn a_machine_that_is_behind_gets_what_it_lacks_and_nothing_is_sent_when_nothing_changed() {
    let (mine, theirs) = (machine(), machine());
    write(&mine, "a", "one", 1_000);
    write(&mine, "b", "two", 2_000);
    let fake = Fake::new(theirs.clone());
    let call = fake.call();
    let mut seen = HashMap::new();
    let now = Instant::now();

    sync_peers(mine.clone(), &[peer("charlie")], &mut seen, &call, now).await;
    assert_eq!(ids(&theirs), ids(&mine));
    assert_eq!(fake.asked(), ["memory.global.list", "memory.global.put", "memory.global.put"]);
    let sent = theirs.bundle_version_latest("b").unwrap().unwrap();
    assert_eq!((sent.written_by.as_str(), sent.source.as_str()), ("synced@narko", "fleet"));

    sync_peers(mine.clone(), &[peer("charlie")], &mut seen, &call, now + Duration::from_secs(15)).await;
    assert_eq!(fake.asked().len(), 3, "an unchanged machine makes no calls at all");
}

#[tokio::test]
async fn an_edit_and_a_removal_here_reach_the_peer_on_the_next_pass_as_just_those() {
    let (mine, theirs) = (machine(), machine());
    write(&mine, "a", "one", 1_000);
    write(&mine, "b", "two", 2_000);
    let fake = Fake::new(theirs.clone());
    let call = fake.call();
    let mut seen = HashMap::new();
    let now = Instant::now();
    sync_peers(mine.clone(), &[peer("charlie")], &mut seen, &call, now).await;
    let before = fake.asked().len();

    write(&mine, "a", "one, edited", 5_000);
    apply_remove(&mine, &FleetRemoval { id: "b".into(), removed_at: 6_000, edited_by: String::new() }, WRITER).unwrap();
    sync_peers(mine.clone(), &[peer("charlie")], &mut seen, &call, now + Duration::from_secs(15)).await;

    assert_eq!(&fake.asked()[before..], ["memory.global.list", "memory.global.remove", "memory.global.put"], "newest first");
    assert_eq!(theirs.bundle_get("a").unwrap().unwrap().instructions, "one, edited");
    assert!(!theirs.bundle_get("b").unwrap().unwrap().is_global);
}

#[tokio::test]
async fn a_peer_that_cannot_be_listed_is_left_alone_until_the_retry_time() {
    let (mine, theirs) = (machine(), machine());
    write(&mine, "a", "one", 1_000);
    let fake = Fake::new(theirs.clone());
    *fake.refuse_list.lock().unwrap() = Some("peer_cannot_do_remote_ops");
    let call = fake.call();
    let mut seen = HashMap::new();
    let now = Instant::now();

    sync_peers(mine.clone(), &[peer("old-build")], &mut seen, &call, now).await;
    sync_peers(mine.clone(), &[peer("old-build")], &mut seen, &call, now + Duration::from_secs(60)).await;
    assert_eq!(fake.asked(), ["memory.global.list"], "asked once, then left alone");

    *fake.refuse_list.lock().unwrap() = None;
    sync_peers(mine.clone(), &[peer("old-build")], &mut seen, &call, now + RETRY + Duration::from_secs(1)).await;
    assert_eq!(ids(&theirs), ids(&mine), "after the retry time it is synced");
}

#[tokio::test]
async fn a_machine_that_went_away_and_came_back_is_synced_again_at_once() {
    let (mine, theirs) = (machine(), machine());
    write(&mine, "a", "one", 1_000);
    let fake = Fake::new(theirs.clone());
    let call = fake.call();
    let mut seen = HashMap::new();
    let now = Instant::now();
    sync_peers(mine.clone(), &[peer("charlie")], &mut seen, &call, now).await;
    let before = fake.asked().len();

    sync_peers(mine.clone(), &[], &mut seen, &call, now + Duration::from_secs(15)).await;
    assert!(seen.is_empty());
    sync_peers(mine.clone(), &[peer("charlie")], &mut seen, &call, now + Duration::from_secs(30)).await;
    assert_eq!(fake.asked().len(), before + 1, "listed again, nothing to send");
}

#[tokio::test]
async fn an_entry_too_big_to_sign_is_skipped_and_the_others_still_go() {
    let (mine, theirs) = (machine(), machine());
    // Under the 1 MiB an entry may be, but every newline is two bytes in the params: over the cap.
    write(&mine, "big", &"
".repeat(700_000), 3_000);
    write(&mine, "medium", &"x".repeat(200_000), 2_000);
    write(&mine, "small", "fits", 1_000);
    let fake = Fake::new(theirs.clone());
    let call = fake.call();
    let mut seen = HashMap::new();

    sync_peers(mine.clone(), &[peer("charlie")], &mut seen, &call, Instant::now()).await;
    assert_eq!(ids(&theirs), [("medium".to_string(), false), ("small".to_string(), false)], "a 200 KB entry goes now");
    assert!(seen.get("id-charlie").unwrap().retry_at.is_none(), "not a failure of the peer");
}

#[tokio::test]
async fn two_machines_each_running_the_loop_end_with_the_same_entries() {
    let (a, b) = (machine(), machine());
    write(&a, "shared", "a wrote this first", 1_000);
    write(&b, "shared", "b wrote this later", 2_000);
    write(&a, "only-a", "a only", 3_000);
    write(&b, "only-b", "b only", 4_000);
    let (a_sees_b, b_sees_a) = (Fake::new(b.clone()), Fake::new(a.clone()));
    let (call_ab, call_ba) = (a_sees_b.call(), b_sees_a.call());
    let (mut seen_a, mut seen_b) = (HashMap::new(), HashMap::new());
    let now = Instant::now();

    sync_peers(a.clone(), &[peer("b")], &mut seen_a, &call_ab, now).await;
    sync_peers(b.clone(), &[peer("a")], &mut seen_b, &call_ba, now).await;
    // The first pass changed what each holds, so each has more to say to the other.
    sync_peers(a.clone(), &[peer("b")], &mut seen_a, &call_ab, now + Duration::from_secs(15)).await;
    sync_peers(b.clone(), &[peer("a")], &mut seen_b, &call_ba, now + Duration::from_secs(15)).await;

    assert_eq!(ids(&a), ids(&b));
    assert_eq!(a.bundle_get("shared").unwrap().unwrap().instructions, "b wrote this later");
    let quiet = a_sees_b.asked().len() + b_sees_a.asked().len();
    sync_peers(a.clone(), &[peer("b")], &mut seen_a, &call_ab, now + Duration::from_secs(30)).await;
    sync_peers(b.clone(), &[peer("a")], &mut seen_b, &call_ba, now + Duration::from_secs(30)).await;
    assert_eq!(a_sees_b.asked().len() + b_sees_a.asked().len(), quiet, "converged: nothing more is sent");
}
