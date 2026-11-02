// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent that starts a turn is wanted running. A stop records `agent:desired = stopped` on the
//! agent's block (`agent_attach::set_desired`) and leaves its controller registered, so a message that
//! comes later is taken by that controller, which runs a turn again with no start in between:
//! nothing set the agent back to `running`, and its pane kept saying "This agent is stopped" over a
//! turn that was working (`AgentStoppedBanner`). A start through `agent.start`, an open or `jekt_start`
//! still sets it; this is the same rule for the path that has no start.
//!
//! Only a turn that begins after the stop counts. A turn that was already running when the stop was
//! recorded (an agent quitting itself records it first, then closes) keeps publishing, and none of those
//! publishes is a new turn.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::backend::mps::{Broker, MuxEvent, EVENT_AGENT_TURN};
use crate::backend::obj::Block;
use crate::backend::storage::store::Store;

use super::app_api::agent_attach::{desired_of, set_desired, DESIRED_RUNNING, DESIRED_STOPPED};

/// The block and the turn of an `agentturn` event whose turn is running.
fn running_turn(event: &str, data: Option<&Value>) -> Option<(String, u64)> {
    if event != EVENT_AGENT_TURN {
        return None;
    }
    let d = data?;
    if d.get("end").is_some() || d.get("active").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some((d.get("block_id")?.as_str()?.to_string(), d.get("turn_id")?.as_u64()?))
}

/// True the first time `turn_id` is seen for `block_id`; a turn's later publishes, and an older id
/// arriving late, are not new.
fn is_new_turn(last: &mut HashMap<String, u64>, block_id: &str, turn_id: u64) -> bool {
    if last.get(block_id).is_some_and(|seen| *seen >= turn_id) {
        return false;
    }
    last.insert(block_id.to_string(), turn_id);
    true
}

/// Record `block_id`'s agent as wanted running when it is recorded as stopped and a turn is running in
/// its controller now. Returns whether it changed. BLOCKING.
fn wake(mstore: &Store, block_id: &str) -> bool {
    let Some(block) = mstore.get::<Block>(block_id).ok().flatten() else { return false };
    if desired_of(&block).as_deref() != Some(DESIRED_STOPPED) {
        return false;
    }
    // The turn must still be running: a stop in progress ends it before it records `stopped`.
    let working = crate::backend::blockcontroller::get_controller(block_id).is_some_and(|c| c.get_runtime_status().turn_active);
    if !working {
        return false;
    }
    set_desired(mstore, block_id, DESIRED_RUNNING);
    tracing::info!(block_id, "a turn started on an agent recorded as stopped: it is wanted running again");
    true
}

/// Start listening. Called once from `main`.
pub(crate) fn install(broker: Arc<Broker>, mstore: Arc<Store>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, u64)>();
    // Observers run inside `Broker::publish`, under its locks: parse and hand off only.
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if let Some(turn) = running_turn(&ev.event, ev.data.as_ref()) {
            let _ = tx.send(turn);
        }
    }));
    tokio::spawn(async move {
        let mut last = HashMap::new();
        while let Some((block_id, turn_id)) = rx.recv().await {
            if !is_new_turn(&mut last, &block_id, turn_id) {
                continue;
            }
            let mstore = Arc::clone(&mstore);
            let _ = tokio::task::spawn_blocking(move || wake(&mstore, &block_id)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_state::test_support::register_stub;
    use crate::backend::blockcontroller::{delete_controller, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING};
    use serde_json::json;

    fn block_with(store: &Store, desired: Option<&str>) -> String {
        let mut block = Block::default();
        block.oid = uuid::Uuid::new_v4().to_string();
        if let Some(d) = desired {
            block.meta.insert("agent:desired".into(), json!(d));
        }
        store.insert(&mut block).unwrap();
        block.oid
    }

    fn desired(store: &Store, id: &str) -> Option<String> {
        desired_of(&store.get::<Block>(id).unwrap().unwrap())
    }

    #[test]
    fn only_a_running_turn_of_the_turn_event_counts() {
        let turn = |end: Option<&str>, active: bool| {
            let mut d = json!({ "block_id": "b1", "turn_id": 7, "active": active });
            if let Some(e) = end {
                d["end"] = json!(e);
            }
            d
        };
        assert_eq!(running_turn("agentturn", Some(&turn(None, true))), Some(("b1".into(), 7)));
        assert_eq!(running_turn("agentturn", Some(&turn(None, false))), None, "between passes");
        assert_eq!(running_turn("agentturn", Some(&turn(Some("completed"), false))), None, "ended");
        assert_eq!(running_turn("agentstatus", Some(&turn(None, true))), None, "another event");
    }

    #[test]
    fn a_turn_is_new_once_and_an_older_one_arriving_late_is_not() {
        let mut last = HashMap::new();
        assert!(is_new_turn(&mut last, "b", 5));
        assert!(!is_new_turn(&mut last, "b", 5), "the same turn's next publish");
        assert!(!is_new_turn(&mut last, "b", 4), "an older turn");
        assert!(is_new_turn(&mut last, "b", 6), "the next turn");
        assert!(is_new_turn(&mut last, "other", 1), "each block has its own");
    }

    /// The stopped banner over a working agent: a stop leaves the controller, a message runs a turn
    /// in it, and the agent is wanted running again; one that is running, or not working, is left.
    #[test]
    fn a_turn_on_an_agent_recorded_as_stopped_records_it_as_running() {
        let store = Store::open_in_memory().unwrap();

        let stopped = block_with(&store, Some("stopped"));
        register_stub(&stopped, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, true);
        assert!(wake(&store, &stopped));
        assert_eq!(desired(&store, &stopped).as_deref(), Some("running"));
        delete_controller(&stopped);

        let idle = block_with(&store, Some("stopped"));
        register_stub(&idle, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
        assert!(!wake(&store, &idle), "no turn running: a stop in progress, or an old publish");
        assert_eq!(desired(&store, &idle).as_deref(), Some("stopped"));
        delete_controller(&idle);

        let no_controller = block_with(&store, Some("stopped"));
        assert!(!wake(&store, &no_controller), "nothing is running it");

        let running = block_with(&store, Some("running"));
        register_stub(&running, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, true);
        assert!(!wake(&store, &running), "already wanted running");
        delete_controller(&running);

        assert!(!wake(&store, &block_with(&store, None)), "nothing recorded");
    }
}
