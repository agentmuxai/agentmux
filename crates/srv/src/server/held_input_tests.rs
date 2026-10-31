// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::obj::MetaMapType;

fn meta(held: Value, parked: bool) -> MetaMapType {
    let mut m = MetaMapType::new();
    m.insert("agentId".into(), json!("ag"));
    m.insert(HELD_INPUT_META_KEY.into(), held);
    if parked {
        m.insert(PARKED_META_KEY.into(), json!(true));
    }
    m
}

fn held(messages: Value) -> Value {
    json!({ "agentId": "ag", "messages": messages })
}

fn msg(id: &str) -> Value {
    json!({ "id": id, "text": format!("text {id}"), "heldAt": 1 })
}

fn status(state: Reachability, since_ms: i64) -> ServiceStatus {
    ServiceStatus { service: "model_api:anthropic".into(), label: "Anthropic API".into(), state, since_ms, last_error: None }
}

fn block_with(store: &Store, meta: MetaMapType) -> String {
    let mut block = Block::default();
    block.oid = uuid::Uuid::new_v4().to_string();
    block.meta = meta;
    store.insert(&mut block).unwrap();
    block.oid
}

fn read(store: &Store, id: &str) -> Block {
    store.get::<Block>(id).unwrap().unwrap()
}

#[test]
fn the_oldest_message_goes_unless_it_may_have_gone_already() {
    let m = meta(held(json!([msg("a"), msg("b")])), true);
    assert_eq!(sendable(&m).unwrap()["id"], "a");
    let mut maybe_gone = msg("a");
    maybe_gone["unconfirmed"] = json!(true);
    let m = meta(held(json!([maybe_gone, msg("b")])), true);
    assert!(sendable(&m).is_none(), "the operator decides on one that may have gone, and on everything after it");
}

#[test]
fn only_messages_typed_to_this_blocks_agent_are_sent() {
    let mut m = meta(json!({ "agentId": "someone-else", "messages": [msg("a")] }), true);
    assert!(sendable(&m).is_none(), "a pane switched to another agent never sends them there");
    m.insert("agentId".into(), json!("someone-else"));
    assert!(sendable(&m).is_some());
    assert!(sendable(&meta(held(json!([])), true)).is_none());
    assert!(sendable(&MetaMapType::new()).is_none());
}

#[test]
fn the_api_holds_input_as_a_pane_decides() {
    let now = 1_000_000;
    assert!(api_allows(None, now), "a service srv doesn't track holds nothing");
    assert!(api_allows(Some(&status(Reachability::Online, 0)), now));
    assert!(api_allows(Some(&status(Reachability::Degraded, 0)), now));
    assert!(!api_allows(Some(&status(Reachability::Offline, 0)), now));
    assert!(!api_allows(Some(&status(Reachability::Unknown, now - 5_000)), now), "not checked yet, for a few seconds");
    assert!(api_allows(Some(&status(Reachability::Unknown, now - 30_000)), now), "not checked for good: don't hold everything");
}

#[test]
fn a_claim_marks_the_message_and_only_while_the_block_is_parked() {
    let store = Store::open_in_memory().unwrap();
    let id = block_with(&store, meta(held(json!([msg("a"), msg("b")])), true));
    assert!(claim(&store, &id, "a", "srv_1").unwrap());
    let first = read(&store, &id).meta[HELD_INPUT_META_KEY]["messages"][0].clone();
    assert_eq!((first["unconfirmed"].clone(), first["attempt"].clone()), (json!(true), json!("srv_1")));
    assert!(!claim(&store, &id, "a", "srv_2").unwrap(), "claimed once");

    // A pane attached in between: the parked mark is gone, so nothing is claimed.
    let attached = block_with(&store, meta(held(json!([msg("a")])), false));
    assert!(!claim(&store, &attached, "a", "srv_1").unwrap());
    assert!(read(&store, &attached).meta[HELD_INPUT_META_KEY]["messages"][0].get("unconfirmed").is_none());

    // Only the first message, and only the one asked for.
    let two = block_with(&store, meta(held(json!([msg("a"), msg("b")])), true));
    assert!(!claim(&store, &two, "b", "srv_1").unwrap());
}

#[test]
fn an_accepted_message_leaves_the_list_and_an_empty_list_leaves_the_meta() {
    let store = Store::open_in_memory().unwrap();
    let id = block_with(&store, meta(held(json!([msg("a"), msg("b")])), true));
    assert!(settle(&store, &id, "a").unwrap());
    assert_eq!(read(&store, &id).meta[HELD_INPUT_META_KEY]["messages"].as_array().unwrap().len(), 1);
    assert!(settle(&store, &id, "b").unwrap());
    assert!(read(&store, &id).meta.get(HELD_INPUT_META_KEY).is_none());
    assert!(!settle(&store, &id, "b").unwrap(), "settled once");
}

#[test]
fn a_send_that_failed_is_held_again_but_a_later_send_is_left_alone() {
    let store = Store::open_in_memory().unwrap();
    let id = block_with(&store, meta(held(json!([msg("a")])), true));
    claim(&store, &id, "a", "srv_1").unwrap();
    assert!(!release(&store, &id, "a", "srv_other").unwrap(), "another attempt's mark is not ours to clear");
    assert!(release(&store, &id, "a", "srv_1").unwrap());
    assert_eq!(read(&store, &id).meta[HELD_INPUT_META_KEY]["messages"][0]["unconfirmed"], json!(false));
}
