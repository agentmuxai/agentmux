// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent whose pane is open but whose controller is not there yet (every resumed pane, for seconds,
//! when AgentMux starts): what is held for it waits uncounted, and a new message is held, not failed.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::backend::reactive::controller_wait::{testing, NO_CONTROLLER};
use crate::backend::reactive::types::SenderDelivery;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;

/// A state with one agent registered by its pane, on a block with no controller yet.
struct Starting {
    state: AppState,
    uid: String,
    slug: String,
    block_id: String,
    /// Whether the delivery path finds a controller on the block.
    controller: Arc<AtomicBool>,
    /// How often the delivery path was asked.
    asked: Arc<AtomicUsize>,
    deliveries: Arc<Mutex<Vec<String>>>,
}

fn starting_agent() -> Starting {
    let mut state = test_state();
    state.reactive_handler = Box::leak(Box::new(crate::backend::reactive::handler::ReactiveHandler::new()));
    let (controller, asked, deliveries) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())));
    let (has, count, received) = (controller.clone(), asked.clone(), deliveries.clone());
    state.reactive_handler.set_message_sender(Arc::new(move |block_id, message, _, _origin, _expires_at_ms| {
        count.fetch_add(1, Ordering::SeqCst);
        if !has.load(Ordering::SeqCst) {
            return Err(format!("{NO_CONTROLLER} {block_id}"));
        }
        received.lock().unwrap().push(message.to_string());
        Ok(SenderDelivery::Delivered)
    }));
    let slug = format!("starting-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let uid = format!("uid-{slug}");
    let mut def = test_agent_def(&uid, "Starting Y", "claude", "agent", 1, "");
    def.slug = slug.clone();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let mut block = crate::backend::obj::Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    state.mstore.insert(&mut block).unwrap();
    state.reactive_handler.register_agent_full(&slug, &block.oid, None, 0, None, Some(&uid), "test").unwrap();
    testing::without_controller(&block.oid);
    Starting { state, uid, slug, block_id: block.oid, controller, asked, deliveries }
}

fn request(slug: &str, id: &str) -> InjectionRequest {
    InjectionRequest {
        target_agent: slug.to_string(),
        message: "hi".to_string(),
        source_agent: Some("sender".to_string()),
        request_id: Some(id.to_string()),
        priority: Some("urgent".to_string()),
        delivery_tier: Some("host".to_string()),
        ..Default::default()
    }
}

fn attempts(t: &Starting, id: &str) -> Option<i64> {
    t.state.mstore.jekt_held_for_target(&t.uid, 10).unwrap().into_iter().find(|row| row.request_id == id).map(|row| row.attempts)
}

/// What a restart into 0.60.24 did: each agent registering replayed a held message at a pane
/// with no controller, and each failure counted toward the twenty that drop it.
#[tokio::test]
async fn a_held_message_is_not_tried_or_counted_while_the_pane_has_no_controller() {
    let t = starting_agent();
    let mut row = crate::server::jekt_hold::held_row(&request(&t.slug, "req-held"), "req-held".to_string(), agentmux_common::time::now_ms());
    row.target_uid = t.uid.clone();
    t.state.mstore.jekt_held_insert(&row).unwrap();

    // Other agents register one after another: every pass leaves it alone.
    for _ in 0..25 {
        let outcomes = replay_pass(&t.state).await;
        assert!(outcomes.iter().all(|(id, _)| id != "req-held"), "{outcomes:?}");
    }
    assert_eq!(t.asked.load(Ordering::SeqCst), 0, "no delivery was tried");
    assert_eq!(attempts(&t, "req-held"), Some(0), "nothing counted, so it can't be dropped for it");

    // The look says there is a controller, the delivery finds none (it went in between): a wait, uncounted.
    testing::controller_arrived(&t.block_id);
    let outcomes = replay_pass(&t.state).await;
    assert!(outcomes.contains(&("req-held".to_string(), ReplayOutcome::Deferred)), "{outcomes:?}");
    assert_eq!(attempts(&t, "req-held"), Some(0));

    // The controller is there: delivered.
    t.controller.store(true, Ordering::SeqCst);
    let outcomes = replay_pass(&t.state).await;
    assert!(outcomes.contains(&("req-held".to_string(), ReplayOutcome::Delivered)), "{outcomes:?}");
    assert_eq!(attempts(&t, "req-held"), None, "delivered rows are gone");
    let deliveries = t.deliveries.lock().unwrap();
    assert_eq!(deliveries.len(), 1);
    assert!(deliveries[0].contains("MSGID=req-held"), "{deliveries:?}");
}

/// A message sent in that window is kept and delivered when the controller registers, where the hold
/// applies; a caller it never serves gets the error, which says to try again.
#[tokio::test]
async fn a_message_sent_while_the_pane_has_no_controller_is_held_until_it_registers() {
    let t = starting_agent();
    let v = crate::server::reactive::deliver(&t.state, crate::server::ReactiveAuthVia::FullAuthKey, "", request(&t.slug, "req-new")).await;
    assert_eq!(v["held"], true, "{v}");
    assert_eq!(v["success"], false, "held is not delivered: {v}");
    assert_eq!(v["held_reason"], "starting", "{v}");
    assert_eq!(v["starting"], true, "{v}");
    assert_eq!(v["retry_when"], "start", "{v}");
    assert!(v["error"].as_str().unwrap().contains("delivered as soon as it is up"), "{v}");
    assert_eq!(attempts(&t, "req-new"), Some(0));

    // An unproven LAN caller is never held: it keeps the error, which is one to retry.
    let lan = InjectionRequest { delivery_tier: Some("lan".to_string()), ..request(&t.slug, "req-lan") };
    let v = crate::server::reactive::deliver(&t.state, crate::server::ReactiveAuthVia::LanKey, "", lan).await;
    assert!(v.get("held").is_none(), "{v}");
    assert!(crate::backend::reactive::is_transient_delivery_error(v["error"].as_str().unwrap()), "{v}");
    assert_eq!(attempts(&t, "req-lan"), None);

    t.controller.store(true, Ordering::SeqCst);
    testing::controller_arrived(&t.block_id);
    let outcomes = replay_pass(&t.state).await;
    assert!(outcomes.contains(&("req-new".to_string(), ReplayOutcome::Delivered)), "{outcomes:?}");
    assert_eq!(t.deliveries.lock().unwrap().len(), 1);
}
