// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The §2.3 table row by row (SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md §5,
//! "Verifier"), against a stub directory.

use std::sync::{Arc, Mutex};

use base64::Engine as _;

use super::*;
use crate::backend::storage::wan_identity::WanIdentityStore;

const NOW: i64 = 1_790_000_000;

/// A stub directory: serves one record (or a status), and counts requests.
struct Stub {
    url: String,
    record_hits: Arc<Mutex<u32>>,
    instance_hits: Arc<Mutex<u32>>,
    _guard: tokio_util::sync::DropGuard,
}

#[derive(Clone)]
enum Serve {
    Record(WanKeyRecord),
    Status(axum::http::StatusCode),
}

async fn stub(serve: Serve, revoked: bool) -> Stub {
    let record_hits = Arc::new(Mutex::new(0u32));
    let instance_hits = Arc::new(Mutex::new(0u32));
    let (rh, ih) = (record_hits.clone(), instance_hits.clone());
    let app = axum::Router::new()
        .route(
            "/agents/:agent_id/wan-key",
            axum::routing::get(move || {
                let serve = serve.clone();
                let rh = rh.clone();
                async move {
                    *rh.lock().unwrap() += 1;
                    match serve {
                        Serve::Record(r) => (axum::http::StatusCode::OK, serde_json::to_string(&r).unwrap()),
                        Serve::Status(s) => (s, "{}".to_string()),
                    }
                }
            }),
        )
        .route(
            "/wan-instances/:instance_id",
            axum::routing::get(move || {
                let ih = ih.clone();
                async move {
                    *ih.lock().unwrap() += 1;
                    (
                        axum::http::StatusCode::OK,
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        format!(r#"{{"instance_id":"x","revoked":{revoked}}}"#),
                    )
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let child = token.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).with_graceful_shutdown(async move { child.cancelled().await }).await;
    });
    Stub { url: format!("http://{addr}"), record_hits, instance_hits, _guard: token.drop_guard() }
}

/// The sending install (camper on narko) and the receiving one (agent2).
struct World {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
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
    World { _dirs: (sd, rd), receiver, receiver_instance, record, private_key }
}

impl World {
    fn row(&self, msg_id: &str, message: &str) -> CarriedRow {
        self.row_at(msg_id, message, NOW)
    }

    fn row_at(&self, msg_id: &str, message: &str, ts: i64) -> CarriedRow {
        let sig = jekt_sign::sign_wan_jekt(
            &self.private_key, msg_id, "camper", &self.record.instance_id, "stable", "agent2", ts, message,
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

    async fn verify(&self, stub: &Stub, row: &CarriedRow, polled: &str, message: &str, now: i64) -> WanVerdict {
        let budget = FetchBudget::new(100);
        let http = reqwest::Client::new();
        let dir = Directory { base_url: &stub.url, http: &http, token: "tok", budget: &budget };
        verify(&self.receiver, &self.receiver_instance, polled, "camper", row, message, now, &dir).await
    }
}

#[tokio::test]
async fn a_valid_same_account_signature_verifies_as_a_new_instance_and_is_cached() {
    let w = world();
    let s = stub(Serve::Record(w.record.clone()), false).await;
    let v = w.verify(&s, &w.row("msg-1", "hello"), "agent2", "hello", NOW).await;
    assert_eq!(v.verified, Some(true), "{v:?}");
    let instance = v.instance.unwrap();
    assert_eq!(instance.id, w.record.instance_id);
    assert_eq!(instance.status, WanInstanceStatus::New, "another install, nobody approved it");
    assert_eq!(instance.label, format!("narko~{}", &w.record.instance_id[..8]));
    assert_eq!(*s.record_hits.lock().unwrap(), 1);
    assert_eq!(*s.instance_hits.lock().unwrap(), 1, "revocation is checked on first sight");

    // A second message from the same key uses the cache, and the revocation
    // check isn't due again yet.
    let v2 = w.verify(&s, &w.row("msg-2", "again"), "agent2", "again", NOW).await;
    assert_eq!(v2.verified, Some(true));
    assert_eq!(*s.record_hits.lock().unwrap(), 1, "self-certifying records are cached");
    assert_eq!(*s.instance_hits.lock().unwrap(), 1);
}

#[tokio::test]
async fn this_installs_own_instance_is_approved() {
    let w = world();
    // The receiver verifying a message signed by its own install.
    let own = w.receiver.instance_ensure("area54").unwrap();
    let key = w.receiver.agent_key_ensure("camper", None).unwrap();
    let record = crate::muxbus::wan_publish::certify(&own, "camper", "stable", &key).unwrap();
    let private = base64::engine::general_purpose::STANDARD.decode(&key.private_key).unwrap();
    let sig = jekt_sign::sign_wan_jekt(&private, "m", "camper", &own.instance_id, "stable", "agent2", NOW, "hi").unwrap();
    let row = CarriedRow {
        wan_sig: Some(sig),
        wan_msg_id: Some("m".into()),
        wan_ts_secs: Some(NOW),
        wan_source_agent: Some("camper".into()),
        wan_target_agent: Some("agent2".into()),
        wan_source_host: Some(own.instance_id.clone()),
        wan_source_channel: Some("stable".into()),
        wan_key_fp: Some(record.key_fp.clone()),
        sender_same_account: Some(true),
    };
    let s = stub(Serve::Record(record), false).await;
    let v = w.verify(&s, &row, "agent2", "hi", NOW).await;
    assert_eq!(v.instance.map(|i| i.status), Some(WanInstanceStatus::Approved));
}

#[tokio::test]
async fn unsigned_and_cross_account_messages_are_none() {
    let w = world();
    let s = stub(Serve::Record(w.record.clone()), false).await;
    let unsigned = CarriedRow { sender_same_account: Some(true), ..Default::default() };
    assert_eq!(w.verify(&s, &unsigned, "agent2", "hello", NOW).await, WanVerdict::default());

    for same in [Some(false), None] {
        let row = CarriedRow { sender_same_account: same, ..w.row("msg-1", "hello") };
        let v = w.verify(&s, &row, "agent2", "hello", NOW).await;
        assert_eq!(v.verified, None, "{same:?}");
    }
    assert_eq!(*s.record_hits.lock().unwrap(), 0, "no directory lookup for a message we won't verify");
}

#[tokio::test]
async fn a_message_moved_to_another_agents_queue_fails() {
    let w = world();
    let s = stub(Serve::Record(w.record.clone()), false).await;
    let v = w.verify(&s, &w.row("msg-1", "hello"), "lark", "hello", NOW).await;
    assert_eq!(v, WanVerdict::failed("wan_envelope_mismatch"));
}

#[tokio::test]
async fn a_stale_message_is_none_not_a_forgery() {
    let w = world();
    let s = stub(Serve::Record(w.record.clone()), false).await;
    let v = w.verify(&s, &w.row("msg-1", "hello"), "agent2", "hello", NOW + 2_101).await;
    assert_eq!(v, WanVerdict::none("wan_sig_stale"));
}

#[tokio::test]
async fn no_record_is_none() {
    let w = world();
    let s = stub(Serve::Status(axum::http::StatusCode::NOT_FOUND), false).await;
    assert_eq!(w.verify(&s, &w.row("msg-1", "hello"), "agent2", "hello", NOW).await, WanVerdict::none("wan_key_not_found"));
}

#[tokio::test]
async fn an_unavailable_directory_is_retried_once_then_none() {
    let w = world();
    let s = stub(Serve::Status(axum::http::StatusCode::SERVICE_UNAVAILABLE), false).await;
    let v = w.verify(&s, &w.row("msg-1", "hello"), "agent2", "hello", NOW).await;
    assert_eq!(v, WanVerdict::none("wan_key_unavailable"));
    assert_eq!(*s.record_hits.lock().unwrap(), 2, "one immediate retry");
}

#[tokio::test]
async fn an_exhausted_fetch_budget_is_unavailable() {
    let w = world();
    let s = stub(Serve::Record(w.record.clone()), false).await;
    let budget = FetchBudget::new(0);
    let http = reqwest::Client::new();
    let dir = Directory { base_url: &s.url, http: &http, token: "tok", budget: &budget };
    let v = verify(&w.receiver, &w.receiver_instance, "agent2", "camper", &w.row("m", "hello"), "hello", NOW, &dir).await;
    assert_eq!(v, WanVerdict::none("wan_key_unavailable"));
    assert_eq!(*s.record_hits.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_tampered_directory_record_is_an_active_failure_and_not_cached() {
    let w = world();
    let mut tampered = w.record.clone();
    tampered.host_hint = "evil".into();
    let s = stub(Serve::Record(tampered), false).await;
    let v = w.verify(&s, &w.row("msg-1", "hello"), "agent2", "hello", NOW).await;
    assert_eq!(v, WanVerdict::failed("wan_cert_invalid"));
    assert!(w.receiver.peer_record_get(&w.record.instance_id, "camper", "stable", &w.record.key_fp).unwrap().is_none());
}

#[tokio::test]
async fn a_cloud_substituting_its_own_key_fails() {
    // A record with a valid chain — but for a key the cloud controls, under
    // an instance it minted. The carried instance and fingerprint don't match.
    let w = world();
    let dir = tempfile::tempdir().unwrap();
    let rogue = WanIdentityStore::open(&dir.path().join("wan.db")).unwrap();
    let rogue_instance = rogue.instance_ensure("narko").unwrap();
    let rogue_key = rogue.agent_key_ensure("camper", None).unwrap();
    let rogue_record = crate::muxbus::wan_publish::certify(&rogue_instance, "camper", "stable", &rogue_key).unwrap();
    let s = stub(Serve::Record(rogue_record), false).await;
    let v = w.verify(&s, &w.row("msg-1", "hello"), "agent2", "hello", NOW).await;
    assert_eq!(v, WanVerdict::failed("wan_record_mismatch"));
}

#[tokio::test]
async fn a_tampered_message_fails_its_signature() {
    let w = world();
    let s = stub(Serve::Record(w.record.clone()), false).await;
    let v = w.verify(&s, &w.row("msg-1", "hello"), "agent2", "hello, but edited", NOW).await;
    assert_eq!(v, WanVerdict::failed("wan_sig_invalid"));
}

#[tokio::test]
async fn a_delivered_signature_replayed_is_an_active_failure() {
    let w = world();
    let s = stub(Serve::Record(w.record.clone()), false).await;
    let row = w.row("msg-1", "hello");
    let first = w.verify(&s, &row, "agent2", "hello", NOW).await;
    assert_eq!(first.verified, Some(true));
    // Not recorded until delivery succeeds: a release-and-redeliver of the
    // same row is not a replay.
    assert_eq!(w.verify(&s, &row, "agent2", "hello", NOW).await.verified, Some(true));
    record_delivered(&w.receiver, &first, &row, NOW);
    assert_eq!(w.verify(&s, &row, "agent2", "hello", NOW).await, WanVerdict::failed("wan_sig_replay"));

    // The replay row outlives the freshness window, not the other way round:
    // at the last second the message could still verify, it's still seen.
    let last = NOW + jekt_sign::WAN_SIG_MAX_AGE_SECS;
    record_delivered(&w.receiver, &first, &w.row("other", "x"), last);
    assert_eq!(w.verify(&s, &row, "agent2", "hello", last).await, WanVerdict::failed("wan_sig_replay"));
}

#[tokio::test]
async fn a_revoked_instance_verifies_but_reads_revoked_and_stays_revoked() {
    let w = world();
    let s = stub(Serve::Record(w.record.clone()), true).await;
    let v = w.verify(&s, &w.row("msg-1", "hello"), "agent2", "hello", NOW).await;
    assert_eq!(v.verified, Some(true));
    assert_eq!(v.instance.unwrap().status, WanInstanceStatus::Revoked);

    // Sticky: a later check saying "not revoked" never clears it.
    let s2 = stub(Serve::Record(w.record.clone()), false).await;
    let t = NOW + 2 * REVOCATION_REFRESH_SECS;
    let later = w.verify(&s2, &w.row_at("msg-2", "hi", t), "agent2", "hi", t).await;
    assert_eq!(later.instance.unwrap().status, WanInstanceStatus::Revoked);
    assert_eq!(*s2.instance_hits.lock().unwrap(), 0, "a revoked instance is never re-asked");
}

#[tokio::test]
async fn revocation_is_re_checked_once_the_last_check_is_an_hour_old() {
    let w = world();
    let s = stub(Serve::Record(w.record.clone()), false).await;
    w.verify(&s, &w.row("m1", "a"), "agent2", "a", NOW).await;
    assert_eq!(*s.instance_hits.lock().unwrap(), 1);

    let later = NOW + REVOCATION_REFRESH_SECS - 1;
    w.verify(&s, &w.row_at("m2", "b", later), "agent2", "b", later).await;
    assert_eq!(*s.instance_hits.lock().unwrap(), 1, "not yet due");

    let due = NOW + REVOCATION_REFRESH_SECS;
    let v = w.verify(&s, &w.row_at("m3", "c", due), "agent2", "c", due).await;
    assert_eq!(v.verified, Some(true));
    assert_eq!(*s.instance_hits.lock().unwrap(), 2, "re-checked after an hour");
}
