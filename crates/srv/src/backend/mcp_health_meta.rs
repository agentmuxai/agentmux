// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The agent MCP self-heal's state on the pane's block (`agent:mcp_health`), for the surfaces that
//! show a row per agent without opening its pane: the Swarm
//! (`docs/specs/SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md` §4.2). `{ "state": "healing" | "restarting" |
//! "gave_up", "step", "at" }` while the agent's AgentMux tools are down or being repaired; the key is
//! removed when they are back. The CLI's error text is not stored here: it is untrusted and the pane
//! row already shows it.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::backend::eventbus::EventBus;
use crate::backend::mcp_health::Notice;
use crate::backend::storage::store::Store;

/// The block meta key.
pub const META_KEY: &str = "agent:mcp_health";

/// What `notice` puts under [`META_KEY`]: the state while the tools are down, `Null` (the key removed)
/// once they are back. `at` is when this state began.
pub fn meta_value(notice: &Notice, now_ms: u64) -> Value {
    match notice.state {
        "healing" | "restarting" | "gave_up" => json!({ "state": notice.state, "step": notice.step, "at": now_ms }),
        _ => Value::Null,
    }
}

/// Store `notice`'s state on `block_id` and tell the open windows. Nothing when it would change nothing
/// (a retry of the same state, or `ok` with no key to remove).
pub fn persist(block_id: &str, notice: &Notice, store: &Arc<Store>, event_bus: &Option<Arc<EventBus>>) {
    let current = store
        .must_get::<crate::backend::obj::Block>(block_id)
        .ok()
        .and_then(|b| b.meta.get(META_KEY).cloned())
        .filter(|v| !v.is_null());
    let mut next = meta_value(notice, agentmux_common::time::now_ms_u64());
    match (&current, &next) {
        (None, Value::Null) => return,
        // The same state again keeps when it began.
        (Some(old), Value::Object(new)) if old.get("state") == new.get("state") && old.get("step") == new.get("step") => return,
        (Some(old), Value::Object(new)) if old.get("state") == new.get("state") => {
            if let (Some(at), Some(obj)) = (old.get("at"), next.as_object_mut()) {
                obj.insert("at".to_string(), at.clone());
            }
        }
        _ => {}
    }
    let mut update = crate::backend::obj::MetaMapType::new();
    update.insert(META_KEY.to_string(), next);
    match crate::server::service::update_object_meta(store, &format!("block:{block_id}"), &update) {
        Ok(_) => {
            if let Some(bus) = event_bus {
                crate::backend::blockcontroller::core::broadcast_block_update(store, bus, block_id);
            }
        }
        Err(e) => tracing::warn!(block_id = %block_id, error = %e, "could not store agent:mcp_health"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(state: &'static str, step: u8) -> Notice {
        Notice { episode: "e".into(), state, step, error: Some("boom".into()), safe_profile: false }
    }

    #[test]
    fn the_tools_being_down_is_stored_without_the_error_and_being_back_removes_it() {
        assert_eq!(meta_value(&notice("healing", 3), 7), json!({ "state": "healing", "step": 3, "at": 7 }));
        assert_eq!(meta_value(&notice("restarting", 0), 7)["state"], "restarting");
        assert_eq!(meta_value(&notice("gave_up", 0), 7)["state"], "gave_up");
        assert_eq!(meta_value(&notice("ok", 0), 7), Value::Null);
    }

    #[test]
    fn a_state_is_stored_once_keeps_when_it_began_and_is_removed_when_the_tools_are_back() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut block = crate::backend::obj::Block {
            oid: "77777777-7777-7777-7777-777777777777".into(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta: crate::backend::obj::MetaMapType::new(),
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
        let read = || store.must_get::<crate::backend::obj::Block>("77777777-7777-7777-7777-777777777777").unwrap().meta.get(META_KEY).cloned();

        persist("77777777-7777-7777-7777-777777777777", &notice("healing", 1), &store, &None);
        let first = read().unwrap();
        assert_eq!(first["state"], "healing");
        persist("77777777-7777-7777-7777-777777777777", &notice("healing", 2), &store, &None);
        let second = read().unwrap();
        assert_eq!(second["step"], 2);
        assert_eq!(second["at"], first["at"], "still the same episode state: it began when it first did");
        persist("77777777-7777-7777-7777-777777777777", &notice("gave_up", 0), &store, &None);
        assert_eq!(read().unwrap()["state"], "gave_up");

        persist("77777777-7777-7777-7777-777777777777", &notice("ok", 0), &store, &None);
        assert!(read().is_none_or(|v| v.is_null()), "the tools are back: nothing to show");
        persist("77777777-7777-7777-7777-777777777777", &notice("ok", 0), &store, &None);
    }
}
