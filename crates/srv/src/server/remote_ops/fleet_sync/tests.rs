// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Mutex;

use futures_util::future::BoxFuture;

use super::*;
use crate::backend::memory::fleet::{apply_put, apply_remove, Actor, FleetEntry, FleetRemoval};

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
    /// When set, a put of this entry id fails on this machine as an older build's store error does: the
    /// operation ran (the answer carries its audit id) and failed. The bool: whether it carries one.
    fail_put: Arc<Mutex<Option<(String, bool)>>>,
}

impl Fake {
    fn new(store: Arc<Store>) -> Self {
        Self { store, asked: Default::default(), refuse_list: Default::default(), fail_put: Default::default() }
    }

    fn call(&self) -> impl Fn(String, &'static str, Value) -> BoxFuture<'static, RemoteOpResult> {
        let (store, asked, refuse, fail_put) = (self.store.clone(), self.asked.clone(), self.refuse_list.clone(), self.fail_put.clone());
        move |_host, op, params| {
            let (store, asked, refuse, fail_put) = (store.clone(), asked.clone(), refuse.clone(), fail_put.clone());
            Box::pin(async move {
                asked.lock().unwrap().push(op.to_string());
                let actor = Actor { label: "synced@narko", uid: "" };
                match op {
                    "memory.global.list" => {
                        if let Some(reason) = *refuse.lock().unwrap() {
                            return RemoteOpResult::refused(reason);
                        }
                        RemoteOpResult::done(json!({ "entries": fleet::list_meta(&store).unwrap() }))
                    }
                    "memory.global.put" => {
                        let entry: FleetEntry = serde_json::from_value(params).unwrap();
                        if let Some((id, ran)) = fail_put.lock().unwrap().clone() {
                            if id == entry.id {
                                let mut answer = RemoteOpResult::failed("sqlite error: UNIQUE constraint failed: db_bundles.name");
                                if ran {
                                    answer.audit_id = "nonce-1".into();
                                }
                                return answer;
                            }
                        }
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
    let mut v: Vec<_> = fleet::list_meta(store).unwrap().into_iter().map(|m| (m.id, m.removed)).collect();
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
async fn one_entry_that_fails_on_the_peer_is_reported_and_the_rest_still_go() {
    let (mine, theirs) = (machine(), machine());
    write(&mine, "a", "one", 1_000);
    write(&mine, "b", "two", 2_000);
    write(&mine, "c", "three", 3_000);
    let fake = Fake::new(theirs.clone());
    *fake.fail_put.lock().unwrap() = Some(("c".into(), true));
    let call = fake.call();
    let mut seen = HashMap::new();

    // `c` is newest, so it goes first: it used to end the pass before `b` and `a` went.
    let report = status::Status::default();
    sync_peers_with(mine.clone(), &[peer("area54")], &mut seen, &call, Instant::now(), &report).await;
    assert_eq!(fake.asked(), ["memory.global.list", "memory.global.put", "memory.global.put", "memory.global.put"]);
    assert!(theirs.bundle_get("a").unwrap().is_some() && theirs.bundle_get("b").unwrap().is_some());
    assert!(theirs.bundle_get("c").unwrap().is_none());
    let snap = report.snapshot();
    assert_eq!(snap["peers"]["area54"]["sent"], 2);
    assert_eq!(snap["peers"]["area54"]["error"], Value::Null, "the machine answered: it is not unreachable");
    let why = snap["peers"]["area54"]["not_applied"]["c"].as_str().unwrap();
    assert!(why.starts_with("failed: ") && why.contains("UNIQUE constraint failed"), "{why}");
}

#[tokio::test]
async fn an_answer_that_never_ran_still_ends_the_pass_for_that_peer() {
    let (mine, theirs) = (machine(), machine());
    write(&mine, "a", "one", 1_000);
    write(&mine, "b", "two", 2_000);
    let fake = Fake::new(theirs.clone());
    *fake.fail_put.lock().unwrap() = Some(("b".into(), false));
    let call = fake.call();
    let mut seen = HashMap::new();

    let report = status::Status::default();
    sync_peers_with(mine.clone(), &[peer("unreachable")], &mut seen, &call, Instant::now(), &report).await;
    assert_eq!(fake.asked(), ["memory.global.list", "memory.global.put"], "the pass stopped at the first failure");
    let snap = report.snapshot();
    assert!(snap["peers"]["unreachable"]["error"].as_str().unwrap().starts_with("memory.global.put: "));
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

/// A machine that already held "a" when its scope table was made, so it is `machine` there by default.
fn machine_with_default_entry(id: &str, text: &str) -> Arc<Store> {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let bundle: crate::backend::storage::store::Bundle = serde_json::from_value(json!({
        "id": id, "name": format!("Rules {id}"), "instructions": text, "is_global": true,
        "mcp_servers": "[]", "skills": "[]", "updated_at": 100,
    }))
    .unwrap();
    store.bundle_upsert(&bundle).unwrap();
    store.bundle_scope_ensure().unwrap();
    store
}

#[tokio::test]
async fn an_entry_the_peer_keeps_for_its_owner_is_reported_as_not_applied_and_not_counted_as_sent() {
    let (mine, theirs) = (machine(), machine_with_default_entry("a", "their wording"));
    theirs.bundle_scope_set("a", "machine").unwrap();
    write(&mine, "a", "my newer wording", 5_000);
    write(&mine, "b", "fresh", 6_000);
    let fake = Fake::new(theirs.clone());
    let call = fake.call();
    let (mut seen, report) = (HashMap::new(), super::status::Status::default());

    sync_peers_with(mine.clone(), &[peer("charlie")], &mut seen, &call, Instant::now(), &report).await;

    assert_eq!(theirs.bundle_get("a").unwrap().unwrap().instructions, "their wording");
    let snap = report.snapshot();
    assert_eq!(snap["peers"]["charlie"]["sent"], 1, "only b arrived");
    assert_eq!(snap["peers"]["charlie"]["not_applied"], json!({ "a": "machine_scope" }));
    assert_eq!(snap["peers"]["charlie"]["error"], Value::Null);
}

#[tokio::test]
async fn a_peers_default_machine_entry_takes_the_newer_write_and_counts_as_sent() {
    let (mine, theirs) = (machine(), machine_with_default_entry("a", "their wording"));
    write(&mine, "a", "my newer wording", 5_000);
    let fake = Fake::new(theirs.clone());
    let call = fake.call();
    let (mut seen, report) = (HashMap::new(), super::status::Status::default());

    sync_peers_with(mine.clone(), &[peer("charlie")], &mut seen, &call, Instant::now(), &report).await;

    assert_eq!(theirs.bundle_get("a").unwrap().unwrap().instructions, "my newer wording");
    assert_eq!(theirs.bundle_scope_get("a").unwrap(), "fleet");
    let snap = report.snapshot();
    assert_eq!(snap["peers"]["charlie"]["sent"], 1);
    assert_eq!(snap["peers"]["charlie"]["not_applied"], json!({}));
}

#[tokio::test]
async fn a_peer_that_cannot_be_listed_shows_the_reason_in_the_status() {
    let (mine, theirs) = (machine(), machine());
    write(&mine, "a", "one", 1_000);
    let fake = Fake::new(theirs);
    *fake.refuse_list.lock().unwrap() = Some("peer_cannot_do_remote_ops");
    let call = fake.call();
    let (mut seen, report) = (HashMap::new(), super::status::Status::default());

    sync_peers_with(mine, &[peer("old-build")], &mut seen, &call, Instant::now(), &report).await;
    assert_eq!(report.snapshot()["peers"]["old-build"]["error"], "list: peer_cannot_do_remote_ops");
}
