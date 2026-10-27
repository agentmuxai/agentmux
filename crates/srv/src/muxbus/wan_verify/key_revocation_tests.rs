// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A message signed by a key its install retired, against a stub directory
//! that serves the record, the 410 for a retired key, and the revocation feed.

use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use base64::Engine as _;

use super::*;
use crate::backend::storage::wan_identity::WanIdentityStore;

const NOW: i64 = 1_790_000_000;

type Answer = Arc<Mutex<(StatusCode, String)>>;

/// A directory with a settable answer for the key and for the feed, which
/// records the query of each feed request.
struct Stub {
    url: String,
    record: Answer,
    feed: Answer,
    record_hits: Arc<Mutex<u32>>,
    feed_queries: Arc<Mutex<Vec<String>>>,
    _guard: tokio_util::sync::DropGuard,
}

async fn stub(record: String) -> Stub {
    let record: Answer = Arc::new(Mutex::new((StatusCode::OK, record)));
    let feed: Answer =
        Arc::new(Mutex::new((StatusCode::OK, r#"{"revocations":[],"next_since":0,"more":false}"#.to_string())));
    let record_hits = Arc::new(Mutex::new(0u32));
    let feed_queries = Arc::new(Mutex::new(Vec::new()));
    let (r, f, rh, fq) = (record.clone(), feed.clone(), record_hits.clone(), feed_queries.clone());
    let json = |(status, body): (StatusCode, String)| (status, [(axum::http::header::CONTENT_TYPE, "application/json")], body);
    let app = axum::Router::new()
        .route(
            "/agents/:agent_id/wan-key",
            axum::routing::get(move || {
                let (r, rh) = (r.clone(), rh.clone());
                async move {
                    *rh.lock().unwrap() += 1;
                    let answer = r.lock().unwrap().clone();
                    json(answer)
                }
            }),
        )
        .route(
            "/wan-key-revocations",
            axum::routing::get(move |axum::extract::RawQuery(query): axum::extract::RawQuery| {
                let (f, fq) = (f.clone(), fq.clone());
                async move {
                    fq.lock().unwrap().push(query.unwrap_or_default());
                    let answer = f.lock().unwrap().clone();
                    json(answer)
                }
            }),
        )
        .route(
            "/wan-instances/:instance_id",
            axum::routing::get(move || async move { json((StatusCode::OK, r#"{"revoked":false}"#.to_string())) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let child = token.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).with_graceful_shutdown(async move { child.cancelled().await }).await;
    });
    Stub { url: format!("http://{addr}"), record, feed, record_hits, feed_queries, _guard: token.drop_guard() }
}

/// The sending install (camper on narko) and the receiving one (agent2).
struct World {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    sender: WanIdentityStore,
    receiver: WanIdentityStore,
    receiver_instance: String,
    record: WanKeyRecord,
    private_key: Vec<u8>,
}

fn world() -> World {
    let (sd, rd) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let sender = WanIdentityStore::open(&sd.path().join("wan.db")).unwrap();
    let receiver = WanIdentityStore::open(&rd.path().join("wan.db")).unwrap();
    let instance = sender.instance_ensure("narko").unwrap();
    let receiver_instance = receiver.instance_ensure("area54").unwrap().instance_id;
    let key = sender.agent_key_ensure("camper", None).unwrap();
    let record = crate::muxbus::wan_publish::certify(&instance, "camper", "stable", &key).unwrap();
    let private_key = base64::engine::general_purpose::STANDARD.decode(&key.private_key).unwrap();
    World { _dirs: (sd, rd), sender, receiver, receiver_instance, record, private_key }
}

impl World {
    fn row(&self, msg_id: &str, ts: i64) -> CarriedRow {
        let sig = jekt_sign::sign_wan_jekt(
            &self.private_key, msg_id, "camper", &self.record.instance_id, "stable", "agent2", ts, "hello",
        )
        .unwrap();
        CarriedRow {
            wan_sig: Some(sig),
            wan_msg_id: Some(msg_id.into()),
            wan_ts_secs: Some(ts),
            wan_source_agent: Some("camper".into()),
            wan_target_agent: Some("agent2".into()),
            wan_source_host: Some(self.record.instance_id.clone()),
            wan_source_channel: Some("stable".into()),
            wan_key_fp: Some(self.record.key_fp.clone()),
            sender_same_account: Some(true),
        }
    }

    async fn verify(&self, stub: &Stub, msg_id: &str, now: i64) -> WanVerdict {
        let budget = FetchBudget::new(100);
        let http = reqwest::Client::new();
        let dir = Directory { base_url: &stub.url, http: &http, token: "tok", budget: &budget };
        verify(&self.receiver, &self.receiver_instance, "agent2", "camper", &self.row(msg_id, now), "hello", now, &dir).await
    }

    async fn recipient(&self, stub: &Stub, now: i64) -> Result<WanKeyRecord, String> {
        let budget = FetchBudget::new(100);
        let http = reqwest::Client::new();
        let dir = Directory { base_url: &stub.url, http: &http, token: "tok", budget: &budget };
        let r = &self.record;
        recipient_record(&self.receiver, &self.receiver_instance, &dir, "camper", &r.instance_id, "stable", &r.key_fp, now).await
    }

    /// The notice the sending install signs when it replaces this key.
    fn revocation(&self) -> WanKeyRevocation {
        let instance = self.sender.instance_ensure("narko").unwrap();
        WanKeyRevocation::sign(&instance.private_key, "camper", "stable", &self.record.key_fp, NOW - 30).unwrap()
    }
}

fn record_json(w: &World) -> String {
    serde_json::to_string(&w.record).unwrap()
}

fn feed_of(revocations: &[WanKeyRevocation], next_since: i64, more: bool) -> String {
    serde_json::json!({ "revocations": revocations, "next_since": next_since, "more": more }).to_string()
}

fn gone_with(revocation: &WanKeyRevocation) -> String {
    serde_json::json!({ "error": "revoked", "revoked": true, "revocation": revocation }).to_string()
}

#[tokio::test]
async fn a_key_the_feed_revoked_is_refused_even_though_it_was_cached() {
    let w = world();
    let s = stub(record_json(&w)).await;
    assert_eq!(w.verify(&s, "m1", NOW).await.verified, Some(true), "before the install replaces it");
    assert!(w.receiver.peer_record_get(&w.record.instance_id, "camper", "stable", &w.record.key_fp).unwrap().is_some());

    *s.feed.lock().unwrap() = (StatusCode::OK, feed_of(&[w.revocation()], NOW - 20, false));
    let hits = *s.record_hits.lock().unwrap();
    let v = w.verify(&s, "m2", NOW + 6 * 60).await;
    assert_eq!(v, WanVerdict::failed("wan_key_revoked"));
    assert_eq!(*s.record_hits.lock().unwrap(), hits, "the key is not looked up again");
    assert!(
        w.receiver.peer_record_get(&w.record.instance_id, "camper", "stable", &w.record.key_fp).unwrap().is_none(),
        "the cached record is dropped"
    );
    // However fresh the message says it is, and however the feed is read later.
    assert_eq!(w.verify(&s, "m3", NOW + 6 * 60 + 1).await, WanVerdict::failed("wan_key_revoked"));
}

#[tokio::test]
async fn a_directory_that_answers_gone_with_the_installs_notice_is_believed_and_remembered() {
    let w = world();
    let s = stub(String::new()).await;
    *s.record.lock().unwrap() = (StatusCode::GONE, gone_with(&w.revocation()));

    assert_eq!(w.verify(&s, "m1", NOW).await, WanVerdict::failed("wan_key_revoked"));
    assert_eq!(*s.record_hits.lock().unwrap(), 1);
    // Kept: the next message is refused without asking the directory.
    assert_eq!(w.verify(&s, "m2", NOW + 1).await, WanVerdict::failed("wan_key_revoked"));
    assert_eq!(*s.record_hits.lock().unwrap(), 1);
    assert!(w.receiver.revoked_key_exists(&w.record.instance_id, "camper", "stable", &w.record.key_fp).unwrap());
}

#[tokio::test]
async fn a_gone_answer_without_a_notice_the_install_signed_for_this_key_is_not_acted_on() {
    let w = world();
    let s = stub(String::new()).await;
    let other_instance = WanIdentityStore::open(&w._dirs.0.path().join("other.db")).unwrap();
    let rogue = other_instance.instance_ensure("rogue").unwrap();
    let mut other_key = w.revocation();
    other_key.key_fp = "z".repeat(43);
    let mut tampered = w.revocation();
    tampered.revoked_at += 1;
    let bodies = [
        r#"{"revoked":true}"#.to_string(),
        gone_with(&WanKeyRevocation::sign(&rogue.private_key, "camper", "stable", &w.record.key_fp, NOW).unwrap()),
        gone_with(&other_key),
        gone_with(&tampered),
        String::new(),
    ];
    for body in bodies {
        *s.record.lock().unwrap() = (StatusCode::GONE, body.clone());
        let v = w.verify(&s, "m1", NOW).await;
        assert_eq!(v.verified, None, "{body}");
        assert_eq!(v.reason, Some("wan_key_unavailable"), "{body}");
    }
    assert!(!w.receiver.revoked_key_exists(&w.record.instance_id, "camper", "stable", &w.record.key_fp).unwrap());
}

#[tokio::test]
async fn an_entry_in_the_feed_that_does_not_verify_is_ignored() {
    let w = world();
    let s = stub(record_json(&w)).await;
    let mut forged = w.revocation();
    forged.revoked_at += 1;
    *s.feed.lock().unwrap() = (StatusCode::OK, feed_of(&[forged], NOW, false));
    assert_eq!(w.verify(&s, "m1", NOW).await.verified, Some(true));
}

#[tokio::test]
async fn the_feed_is_read_at_most_every_five_minutes_and_from_where_it_left_off() {
    let w = world();
    let s = stub(record_json(&w)).await;
    *s.feed.lock().unwrap() = (StatusCode::OK, feed_of(&[], 4242, false));

    w.verify(&s, "m1", NOW).await;
    w.verify(&s, "m2", NOW + 60).await;
    w.verify(&s, "m3", NOW + 299).await;
    assert_eq!(*s.feed_queries.lock().unwrap(), vec!["since=0".to_string()], "once, from the start");

    w.verify(&s, "m4", NOW + 301).await;
    assert_eq!(*s.feed_queries.lock().unwrap(), vec!["since=0".to_string(), "since=4242".to_string()]);
}

#[tokio::test]
async fn a_revocation_that_could_not_be_stored_is_read_again_from_the_same_place() {
    let w = world();
    let s = stub(record_json(&w)).await;
    *s.feed.lock().unwrap() = (StatusCode::OK, feed_of(&[w.revocation()], 777, false));
    let directory = crate::muxbus::wan_publish::directory_id(&s.url, "");
    let path = w._dirs.1.path().join("wan.db");

    // Storing fails (the table cannot be written).
    rusqlite::Connection::open(&path).unwrap().execute_batch("ALTER TABLE wan_revoked_keys RENAME TO wan_revoked_keys_away;").unwrap();
    w.verify(&s, "m1", NOW).await;
    assert_eq!(w.receiver.key_revocation_feed_state(&directory).unwrap().0, 0, "the cursor did not move past it");

    // The store works again: the next read, an interval later, gets the same entry and keeps it.
    rusqlite::Connection::open(&path).unwrap().execute_batch("ALTER TABLE wan_revoked_keys_away RENAME TO wan_revoked_keys;").unwrap();
    let v = w.verify(&s, "m2", NOW + 6 * 60).await;
    assert_eq!(v, WanVerdict::failed("wan_key_revoked"));
    assert_eq!(s.feed_queries.lock().unwrap().last().unwrap(), "since=0");
    assert_eq!(w.receiver.key_revocation_feed_state(&directory).unwrap().0, 777);
}

#[tokio::test]
async fn pages_of_the_feed_are_followed_and_a_stuck_cursor_stops() {
    let w = world();
    let s = stub(record_json(&w)).await;
    *s.feed.lock().unwrap() = (StatusCode::OK, feed_of(&[], 10, true));
    w.verify(&s, "m1", NOW).await;
    // since=0 → next 10 (more) → since=10 → next 10, no progress: stop.
    assert_eq!(*s.feed_queries.lock().unwrap(), vec!["since=0".to_string(), "since=10".to_string()]);
}

#[tokio::test]
async fn a_cloud_without_the_feed_does_not_stop_verification_and_is_left_alone_for_an_hour() {
    let w = world();
    let s = stub(record_json(&w)).await;
    *s.feed.lock().unwrap() = (StatusCode::NOT_FOUND, "{}".to_string());
    assert_eq!(w.verify(&s, "m1", NOW).await.verified, Some(true));
    w.verify(&s, "m2", NOW + 10 * 60).await;
    w.verify(&s, "m3", NOW + 59 * 60).await;
    assert_eq!(s.feed_queries.lock().unwrap().len(), 1);
    w.verify(&s, "m4", NOW + 61 * 60).await;
    assert_eq!(s.feed_queries.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_feed_that_is_down_is_not_hammered_and_does_not_block_a_message() {
    let w = world();
    let s = stub(record_json(&w)).await;
    *s.feed.lock().unwrap() = (StatusCode::SERVICE_UNAVAILABLE, "{}".to_string());
    assert_eq!(w.verify(&s, "m1", NOW).await.verified, Some(true));
    w.verify(&s, "m2", NOW + 30).await;
    assert_eq!(s.feed_queries.lock().unwrap().len(), 2, "one read with its one retry, then left until the interval is up");
}

#[tokio::test]
async fn a_replaced_agents_new_key_is_not_touched_by_the_old_ones_revocation() {
    let w = world();
    let s = stub(record_json(&w)).await;
    w.receiver.revoked_key_put(&w.revocation(), NOW).unwrap();

    // The install replaces camper's key: new key, same name, instance and channel.
    let instance = w.sender.instance_ensure("narko").unwrap();
    w.sender.agent_keys_delete(&["camper".to_string()]).unwrap();
    let new_key = w.sender.agent_key_ensure("camper", None).unwrap();
    let new_record = crate::muxbus::wan_publish::certify(&instance, "camper", "stable", &new_key).unwrap();
    let renewed = World {
        _dirs: (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()),
        sender: WanIdentityStore::open(&w._dirs.0.path().join("again.db")).unwrap(),
        receiver: WanIdentityStore::open(&w._dirs.1.path().join("wan.db")).unwrap(),
        receiver_instance: w.receiver_instance.clone(),
        record: new_record.clone(),
        private_key: base64::engine::general_purpose::STANDARD.decode(&new_key.private_key).unwrap(),
    };
    *s.record.lock().unwrap() = (StatusCode::OK, serde_json::to_string(&new_record).unwrap());
    assert_eq!(renewed.verify(&s, "m1", NOW).await.verified, Some(true));
    assert_eq!(w.verify(&s, "m2", NOW).await, WanVerdict::failed("wan_key_revoked"), "the old key stays refused");
}

#[tokio::test]
async fn a_file_is_not_sealed_to_a_retired_key() {
    let w = world();
    let s = stub(record_json(&w)).await;
    assert!(w.recipient(&s, NOW).await.is_ok());

    // Learned from the feed.
    *s.feed.lock().unwrap() = (StatusCode::OK, feed_of(&[w.revocation()], NOW, false));
    let err = w.recipient(&s, NOW + 6 * 60).await.unwrap_err();
    assert!(err.contains("retired"), "{err}");

    // Learned from the directory's 410 on a receiver that had heard nothing.
    let fresh = world();
    let s2 = stub(String::new()).await;
    *s2.record.lock().unwrap() = (StatusCode::GONE, gone_with(&fresh.revocation()));
    let err = fresh.recipient(&s2, NOW).await.unwrap_err();
    assert!(err.contains("retired"), "{err}");
    assert!(fresh.receiver.revoked_key_exists(&fresh.record.instance_id, "camper", "stable", &fresh.record.key_fp).unwrap());
}
