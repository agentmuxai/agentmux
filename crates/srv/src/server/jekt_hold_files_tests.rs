// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A verified message from another machine with files, to an agent that can't work: held with its files
//! and delivered with them when the agent can.

use std::sync::{Arc, Mutex};

use super::*;
use crate::backend::agent_state::test_support::register_stub;
use crate::backend::blockcontroller::{core::META_LAST_FAILURE, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING};
use crate::backend::reactive::types::{WanInstanceInfo, WanInstanceStatus};
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;
use agentmux_common::jekt_files::JektFile;

fn file() -> JektFile {
    JektFile { sha256: "ab".repeat(32), name: "notes.txt".into(), size: 11 }
}

fn lan_with_files(verified: Option<bool>) -> InjectionRequest {
    InjectionRequest {
        target_agent: "target".into(),
        message: "see the file".into(),
        source_agent: Some("sender".into()),
        request_id: Some("req-files".into()),
        delivery_tier: Some("lan".into()),
        lan_verified: verified,
        files: Some(vec![file()]),
        ..Default::default()
    }
}

#[test]
fn only_a_verified_sender_with_files_that_are_not_withheld_waits() {
    assert!(peer_files_wait(&lan_with_files(Some(true)), false));
    assert!(!peer_files_wait(&lan_with_files(Some(true)), true), "withheld files are refused before a byte is read");
    assert!(!peer_files_wait(&lan_with_files(None), false), "the LAN key alone proves nothing");
    assert!(!peer_files_wait(&lan_with_files(Some(false)), false));
    assert!(!peer_files_wait(&InjectionRequest { source_agent: Some("cron".into()), ..lan_with_files(Some(true)) }, false));
    assert!(!peer_files_wait(&InjectionRequest { request_id: None, ..lan_with_files(Some(true)) }, false));
}

/// A state with an agent whose last run hit its limit (or a sign-in refusal), and what its replay delivers.
struct Limited {
    state: AppState,
    uid: String,
    slug: String,
    block: crate::backend::obj::Block,
    deliveries: Arc<Mutex<Vec<String>>>,
}

fn limited_agent() -> Limited {
    let mut state = test_state();
    state.reactive_handler = Box::leak(Box::new(crate::backend::reactive::handler::ReactiveHandler::new()));
    let deliveries = Arc::new(Mutex::new(Vec::new()));
    let received = deliveries.clone();
    state.reactive_handler.set_message_sender(Arc::new(move |_, message, _, _origin| {
        received.lock().unwrap().push(message.to_string());
        Ok(crate::backend::reactive::types::SenderDelivery::Delivered)
    }));
    let slug = format!("files-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let uid = format!("uid-{slug}");
    let mut def = test_agent_def(&uid, "Files Y", "claude", "agent", 1, "");
    def.slug = slug.clone();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let mut block = crate::backend::obj::Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    block.meta.insert(
        META_LAST_FAILURE.to_string(),
        serde_json::json!({"code": "usage_limit", "title": "Usage limit reached", "detail": "", "retryable": false, "resetsAtMs": agentmux_common::time::now_ms_u64() + 3_600_000}),
    );
    state.mstore.insert(&mut block).unwrap();
    register_stub(&block.oid, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
    state.reactive_handler.register_agent_full(&slug, &block.oid, None, 0, None, Some(&uid), "test").unwrap();
    Limited { state, uid, slug, block, deliveries }
}

impl Limited {
    /// The limit is over: the agent can work.
    fn recovers(&mut self) {
        self.block.meta.remove(META_LAST_FAILURE);
        self.state.mstore.update(&mut self.block).unwrap();
    }
}

#[tokio::test]
async fn a_lan_message_with_files_to_a_limited_agent_is_held_with_its_files_and_delivered_with_them() {
    let mut t = limited_agent();
    let mut req = InjectionRequest { target_agent: t.slug.clone(), ..lan_with_files(Some(true)) };
    let unavailable = local_target_unavailable(&t.state, &t.slug).await.expect("the agent is out of quota");
    assert!(peer_files_wait(&req, false));

    // What `jekt_files::receive` does once the files are in the store.
    let mut replay = replay::reserve(&mut req, agentmux_common::time::now_secs());
    let held = hold_for_unavailable(&t.state, crate::server::ReactiveAuthVia::LanKey, &req, &unavailable, &mut replay).await;
    assert_eq!(held["held"], true, "{held}");
    assert_eq!(held["held_reason"], "usage_limit", "{held}");
    let rows = t.state.mstore.jekt_held_for_target(&t.uid, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].delivery_tier.as_str(), rows[0].lan_verified), ("lan", Some(true)));
    let kept: Vec<JektFile> = serde_json::from_str(&rows[0].files_json).unwrap();
    assert_eq!(kept, vec![file()], "the row keeps the file list for the replay");

    // Limited still: the replay leaves it. Recovered: it is delivered, and the files are part of the delivery.
    assert!(crate::server::jekt_held::replay_pass(&t.state).await.iter().all(|(id, _)| id != "req-files"));
    t.recovers();
    let outcomes = crate::server::jekt_held::replay_pass(&t.state).await;
    assert!(outcomes.iter().any(|(id, o)| id == "req-files" && *o == crate::server::jekt_held::ReplayOutcome::Delivered), "{outcomes:?}");
    let delivered = t.deliveries.lock().unwrap().clone();
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].contains("MSGID=req-files"), "{}", delivered[0]);
    assert!(delivered[0].contains("1 file sent with this message"), "the replay builds the files' part of the message: {}", delivered[0]);
    crate::backend::blockcontroller::delete_controller(&t.block.oid);
}

#[tokio::test]
async fn an_unverified_lan_message_with_files_is_refused_not_held() {
    let t = limited_agent();
    let mut req = InjectionRequest { target_agent: t.slug.clone(), ..lan_with_files(None) };
    let unavailable = local_target_unavailable(&t.state, &t.slug).await.unwrap();
    let mut replay = replay::reserve(&mut req, agentmux_common::time::now_secs());
    let answer = hold_for_unavailable(&t.state, crate::server::ReactiveAuthVia::LanKey, &req, &unavailable, &mut replay).await;
    assert!(answer.get("held").is_none(), "{answer}");
    assert!(crate::backend::reactive::is_unavailable_refusal(answer["error"].as_str().unwrap()), "{answer}");
    assert!(t.state.mstore.jekt_held_for_target(&t.uid, 10).unwrap().is_empty());
    crate::backend::blockcontroller::delete_controller(&t.block.oid);
}

#[tokio::test]
async fn a_wan_message_with_files_to_a_limited_agent_is_held_with_them() {
    let mut t = limited_agent();
    let req = InjectionRequest {
        target_agent: t.slug.clone(),
        message: "from the other machine".into(),
        source_agent: Some("sender".into()),
        request_id: Some("req-wan-files".into()),
        delivery_tier: Some("wan".into()),
        wan_verified: Some(true),
        wan_instance: Some(WanInstanceInfo { id: "I1".into(), label: "host~I1".into(), status: WanInstanceStatus::Approved }),
        files: Some(vec![file()]),
        ..Default::default()
    };
    assert!(hold_wan_for_unavailable(&t.state.mstore, t.state.reactive_handler, &req), "files no longer send it down the old path");
    let rows = t.state.mstore.jekt_held_for_target(&t.uid, 10).unwrap();
    assert_eq!((rows.len(), rows[0].delivery_tier.as_str()), (1, "wan"));
    assert!(!rows[0].files_json.is_empty());

    // An install not approved is not held, with files or without.
    let new_install = InjectionRequest {
        request_id: Some("req-new".into()),
        wan_instance: Some(WanInstanceInfo { id: "I2".into(), label: "host~I2".into(), status: WanInstanceStatus::New }),
        ..req.clone()
    };
    assert!(!hold_wan_for_unavailable(&t.state.mstore, t.state.reactive_handler, &new_install));

    t.recovers();
    let outcomes = crate::server::jekt_held::replay_pass(&t.state).await;
    assert!(outcomes.iter().any(|(id, _)| id == "req-wan-files"), "{outcomes:?}");
    assert!(t.deliveries.lock().unwrap()[0].contains("1 file sent with this message"));
    crate::backend::blockcontroller::delete_controller(&t.block.oid);
}
