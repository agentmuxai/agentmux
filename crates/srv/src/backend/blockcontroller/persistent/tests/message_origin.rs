// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The record of a delivered message says where it came from and which notes
//! went with it (`message_origin`), also when it waited in the queue, and a
//! message with no origin is never written as the human's.

use super::super::*;
use crate::backend::message_origin::OriginTrust;
use crate::backend::reactive::held_notes;

fn controller_with_store(block_id: &str) -> (Arc<PersistentSubprocessController>, mpsc::Receiver<String>, Arc<FileStore>) {
    let broker = Arc::new(crate::backend::mps::Broker::new());
    let filestore = Arc::new(FileStore::open_in_memory().unwrap());
    let c = Arc::new(PersistentSubprocessController::new(
        "tab".to_string(),
        block_id.to_string(),
        Some(broker),
        None,
        None,
        Some(filestore.clone()),
    ));
    let (tx, rx) = mpsc::channel::<String>(8);
    c.inner.lock().unwrap().stdin_tx = Some(tx);
    (c, rx, filestore)
}

/// The user records in the block's `output`, in order.
fn stored_user_records(filestore: &FileStore, block_id: &str) -> Vec<serde_json::Value> {
    let bytes = filestore.read_file(block_id, PERSISTENT_OUTPUT_SUBJECT).unwrap().unwrap_or_default();
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["type"] == "user")
        .collect()
}

fn jekt_from(name: &str) -> RecordOrigin {
    let mut origin = MessageOrigin::from_sender(name);
    origin.trust = Some(OriginTrust::HostVerified);
    RecordOrigin::new(origin)
}

#[tokio::test]
async fn a_message_sent_to_a_live_agent_is_stored_with_where_it_came_from_and_its_notes() {
    let block_id = "block-origin-direct";
    let (c, _rx, fs) = controller_with_store(block_id);
    let mut record = jekt_from("agent1");
    record.notes = vec!["[AgentMux] AgentMux was upgraded".to_string()];

    c.send_user_message_with_images_from("[JEKT:FROM=agent1 …]".to_string(), None, Vec::new(), record).unwrap();

    let records = stored_user_records(&fs, block_id);
    assert_eq!(records.len(), 1);
    let amx = &records[0]["amx"];
    assert_eq!(amx["v"], 1);
    assert_eq!(amx["origin"]["kind"], "agent");
    assert_eq!(amx["origin"]["from"], "agent1");
    assert_eq!(amx["origin"]["trust"], "host-verified");
    assert_eq!(amx["notes"], serde_json::json!(["[AgentMux] AgentMux was upgraded"]));
    assert_eq!(records[0]["message"]["content"], "[JEKT:FROM=agent1 …]", "the message itself is as it was sent");
}

#[tokio::test]
async fn a_message_that_waited_in_the_queue_keeps_its_own_origin() {
    let block_id = "block-origin-deferred";
    let (c, _rx, fs) = controller_with_store(block_id);
    // The process can't take a write yet, so both wait; they are written in order.
    c.inner.lock().unwrap().spawning_in_progress = true;
    c.send_user_message_with_images_from("from the human".to_string(), None, Vec::new(), RecordOrigin::new(MessageOrigin::user()))
        .unwrap();
    c.send_user_message_with_images_from("from a schedule".to_string(), None, Vec::new(), jekt_from("cron")).unwrap();
    assert!(stored_user_records(&fs, block_id).is_empty(), "nothing is stored before it is delivered");

    c.inner.lock().unwrap().spawning_in_progress = false;
    let mut orphaned = 0;
    c.sweep_deferred_once(&mut orphaned);

    let records = stored_user_records(&fs, block_id);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["amx"]["origin"], serde_json::json!({ "kind": "user" }));
    assert_eq!(records[1]["amx"]["origin"]["kind"], "schedule");
    assert_eq!(records[1]["amx"]["origin"]["from"], "cron");
}

#[test]
fn a_note_that_joins_a_turn_is_stored_as_agentmux() {
    let block_id = "block-origin-note";
    let (c, mut rx, fs) = controller_with_store(block_id);
    c.health_monitor.set_active_turn(true);
    assert!(c.note_into_turn_or_hold("k", "[AgentMux] overlap"));
    assert!(rx.try_recv().unwrap().contains("[AgentMux] overlap"));

    let records = stored_user_records(&fs, block_id);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["amx"]["origin"], serde_json::json!({ "kind": "system" }));
    // Nothing held is left over for the next message.
    let (message, notes) = held_notes::attach(block_id, "");
    notes.delivered();
    assert_eq!(message, "");
}

#[test]
fn a_record_written_with_no_origin_has_no_amx_origin() {
    // The Gemini-style gap: a writer with nothing to say stores the line as it is,
    // which a reader takes as untyped and never as the human's.
    let block_id = "block-origin-none";
    let (c, _rx, fs) = controller_with_store(block_id);
    crate::backend::transcript::append::persist_injected_user_line(
        c.broker.as_deref(),
        block_id,
        b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n",
        Some(&fs),
        None,
        None,
    );
    let records = stored_user_records(&fs, block_id);
    assert_eq!(records.len(), 1);
    assert!(records[0].get("amx").is_none());
}
