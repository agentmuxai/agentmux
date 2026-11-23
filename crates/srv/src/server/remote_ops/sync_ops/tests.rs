// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The sync ops as the transport serves them: nothing for a caller without a certificate, for a stranger or while
// another install holds the computer's lock; a member reads and writes its collections; and the ops are write ops
// with limits of their own.

use serde_json::json;

use super::super::{lookup, RemoteCaller, Verification};
use super::*;
use crate::backend::user_sync::hlc::{Clock, Version};
use crate::backend::user_sync::members::{Change, Policy, Subject};
use crate::backend::user_sync::record::{Header, Record};
use crate::backend::user_sync::keys;
use crate::server::tests::test_state;

const C: &str = "board.t";

/// A state whose install has an identity of its own, as a running one does.
fn with_identity() -> AppState {
    let state = test_state();
    let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let wan = std::sync::Arc::new(wan_identity::WanIdentityStore::open(&dir.path().join("wan.db")).unwrap());
    wan.instance_ensure(&crate::backend::reactive::registry::local_host_label()).unwrap();
    state.mstore.set_wan_identity(wan);
    state
}

fn caller(instance: &str) -> RemoteCaller {
    RemoteCaller {
        agent_uid: "uid-claimed".into(),
        slug: "loap".into(),
        machine: "charlie".into(),
        instance_id: instance.into(),
        install_trusted: true,
        verification: Verification::Lan,
        reason: "sync".into(),
    }
}

struct Peer {
    private: [u8; 32],
    subject: Subject,
}

fn peer() -> Peer {
    let private = [9u8; 32];
    let subject = Subject::machine(&private, &crate::backend::jekt_seal::box_public(&private).unwrap(), "label").unwrap();
    Peer { private, subject }
}

fn me(state: &AppState) -> crate::backend::storage::wan_identity::WanInstance {
    let wan = state.mstore.wan_identity().or_else(wan_identity::global).unwrap();
    wan.instance_ensure(&crate::backend::reactive::registry::local_host_label()).unwrap()
}

/// This install founds a collection of `kind` and adds the peer to it, as the store would hold it.
fn collection_with_peer(state: &AppState, kind: &str, peer: &Peer) {
    let inst = me(state);
    let conn = state.id_store.conn().lock().unwrap();
    log::init(&conn).unwrap();
    let mut chains = std::collections::BTreeMap::new();
    let mine = Subject::machine(&inst.private_key, &crate::backend::jekt_seal::box_public(&inst.private_key).unwrap(), "me").unwrap();
    let store = SqlKeys(&conn);
    let keys = StoreKeys { store: &store, wan_public: inst.public_key, wan_seed: inst.private_key };
    ops::found(&conn, &mut chains, &keys, C, kind, &mine, &inst.private_key, Policy::Approve, agentmux_common::time::now_ms()).unwrap();
    let chain = chains.get_mut(C).unwrap();
    let add = chain.next(&inst.private_key, C, &inst.instance_id, Change::Add { member: peer.subject.clone() }).unwrap();
    chain.take(C, add).unwrap();
    chain_store::save(&conn, C, chain).unwrap();
}

fn write_here(state: &AppState, record: &str, body: &str) {
    let inst = me(state);
    let conn = state.id_store.conn().lock().unwrap();
    let mut clock = Clock::new(&inst.instance_id);
    let mut signer = log::Signer { machine: &inst.instance_id, private: &inst.private_key, clock: &mut clock };
    log::append_local(&conn, &mut signer, C, record, body.as_bytes().to_vec(), false, agentmux_common::time::now_ms()).unwrap();
}

fn from_peer(peer: &Peer, record: &str, seq: u64, body: &str) -> Record {
    let origin = peer.subject.id.clone();
    let header = Header {
        collection: C.into(),
        record: record.into(),
        version: Version { wall_ms: agentmux_common::time::now_ms(), counter: 0, machine: origin.clone() },
        base: None,
        origin,
        seq,
        epoch: 0,
        tombstone: false,
        size: body.len() as u64,
    };
    Record::sign(&peer.private, header, body.as_bytes().to_vec()).unwrap()
}

fn code(r: RemoteResult) -> &'static str {
    r.unwrap_err().code
}

#[tokio::test]
async fn a_caller_with_no_certificate_is_not_answered() {
    let state = with_identity();
    let ask = json!({ "collection": C, "vector": {}, "limit": 10 });
    assert_eq!(code(pull(&state, &caller(""), ask).await), "needs_certificate");
}

#[tokio::test]
async fn a_caller_that_is_not_a_member_is_refused_alike_for_a_collection_that_exists_and_one_that_does_not() {
    let state = with_identity();
    collection_with_peer(&state, "kanban", &peer());
    let ask = |c: &str| json!({ "collection": c, "vector": {}, "limit": 10 });
    assert_eq!(code(pull(&state, &caller("stranger"), ask(C)).await), "not_member");
    assert_eq!(code(pull(&state, &caller("stranger"), ask("board.none")).await), "not_member");
    let held = hello(&state, &caller("stranger"), json!({ "collections": [C] })).await.unwrap();
    assert_eq!(held["held"], json!([]));
}

#[tokio::test]
async fn a_member_reads_what_this_install_holds_and_writes_its_own_records() {
    let state = with_identity();
    let p = peer();
    collection_with_peer(&state, "kanban", &p);
    write_here(&state, "card.1", "one");
    let who = caller(&p.subject.id);
    let held = hello(&state, &who, json!({ "collections": [C] })).await.unwrap();
    assert_eq!(held["held"][0]["collection"], C);
    assert_eq!(held["held"][0]["n"], 2);
    let pulled = pull(&state, &who, json!({ "collection": C, "vector": {}, "limit": 10 })).await.unwrap();
    assert_eq!(pulled["records"].as_array().unwrap().len(), 1);
    assert_eq!(pulled["more"], false);

    let rec = from_peer(&p, "card.2", 1, "two");
    let pushed = push(&state, &who, json!({ "collection": C, "records": [rec] })).await.unwrap();
    assert_eq!(pushed["results"][0], json!({ "Ok": "Applied" }));
    let again = pull(&state, &who, json!({ "collection": C, "vector": { p.subject.id.clone(): 1 }, "limit": 10 })).await.unwrap();
    assert_eq!(again["records"].as_array().unwrap().len(), 1, "only this install's own record is lacking on the peer");
    let entries = members(&state, &who, json!({ "collection": C, "have_n": 0, "offered": [] })).await.unwrap();
    assert_eq!(entries["entries"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn nothing_is_answered_while_another_install_holds_the_computers_lock() {
    let state = with_identity();
    let p = peer();
    collection_with_peer(&state, "kanban", &p);
    {
        let conn = state.id_store.conn().lock().unwrap();
        let taken = lock::acquire(&conn, "another-install", agentmux_common::time::now_ms(), LEASE_MS).unwrap();
        assert!(matches!(taken, Lock::Held { .. }));
    }
    let ask = json!({ "collection": C, "vector": {}, "limit": 10 });
    assert_eq!(code(pull(&state, &caller(&p.subject.id), ask).await), "not_active");
}

#[tokio::test]
async fn a_push_for_a_kind_this_build_does_not_merge_is_refused() {
    let state = with_identity();
    let p = peer();
    collection_with_peer(&state, "vault", &p);
    let rec = from_peer(&p, "secret.1", 1, "x");
    assert_eq!(code(push(&state, &caller(&p.subject.id), json!({ "collection": C, "records": [rec] })).await), "bad_request");
}

#[tokio::test]
async fn a_member_is_given_the_collections_keys_sealed_to_it_and_nobody_else_is() {
    let state = with_identity();
    let p = peer();
    collection_with_peer(&state, "kanban", &p);
    let public = ed25519_dalek::SigningKey::from_bytes(&p.private).verifying_key().to_bytes();
    let who = caller(&p.subject.id);
    let given = super::keys(&state, &who, json!({ "collection": C, "have": [] })).await.unwrap();
    let sealed = given["keys"][0][1].as_str().unwrap();
    assert_eq!(given["keys"][0][0], 0);
    assert!(keys::open_sealed(sealed, &public, &p.private, C, 0).is_some(), "the member opens it");
    assert!(keys::open_sealed(sealed, &public, &[3u8; 32], C, 0).is_none(), "another seed does not");
    assert_eq!(code(super::keys(&state, &caller("stranger"), json!({ "collection": C, "have": [] })).await), "not_member");
    let again = super::keys(&state, &who, json!({ "collection": C, "have": [0] })).await.unwrap();
    assert_eq!(again["keys"], json!([]), "only what the member lacks");
}

#[tokio::test]
async fn a_join_waits_for_a_person_and_is_still_waiting_in_the_next_call() {
    let state = with_identity();
    let p = peer();
    {
        let inst = me(&state);
        let conn = state.id_store.conn().lock().unwrap();
        log::init(&conn).unwrap();
        let mut chains = std::collections::BTreeMap::new();
        let mine = Subject::machine(&inst.private_key, &crate::backend::jekt_seal::box_public(&inst.private_key).unwrap(), "me").unwrap();
        let store = SqlKeys(&conn);
        let keys = StoreKeys { store: &store, wan_public: inst.public_key, wan_seed: inst.private_key };
        ops::found(&conn, &mut chains, &keys, C, "kanban", &mine, &inst.private_key, Policy::Approve, agentmux_common::time::now_ms()).unwrap();
    }
    let who = caller(&p.subject.id);
    let ask = json!({ "collection": C, "subject": p.subject });
    assert_eq!(join(&state, &who, ask.clone()).await.unwrap(), json!("Pending"));
    assert_eq!(join(&state, &who, ask).await.unwrap(), json!("Pending"));
    let conn = state.id_store.conn().lock().unwrap();
    assert_eq!(chain_store::load_pending(&conn).unwrap().len(), 1, "asked twice, waiting once, and kept between calls");
}

#[test]
fn the_sync_ops_are_write_ops_with_limits_of_their_own() {
    for name in ["sync.hello", "sync.pull", "sync.push", "sync.members", "sync.join", "sync.keys"] {
        let op = lookup(name).unwrap_or_else(|| panic!("{name} is registered"));
        assert!(op.changes_state && op.needs_approved_install, "{name} needs a certificate from an approved install");
    }
    assert!(lookup("sync.push").unwrap().max_params >= ops::MAX_WIRE_BYTES);
    assert!(lookup("sync.pull").unwrap().max_params < 64 * 1024);
    assert!(lookup("sync.join").is_some() && lookup("sync.keys").is_some());
}
