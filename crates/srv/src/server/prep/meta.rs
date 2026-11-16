// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The pane meta a pending change shows through. The composer's chip reads it live, and it
//! outlives a window reload, as the queued account switch's own meta does.

use serde_json::{json, Value};

use crate::backend::blockcontroller;
use crate::backend::obj;
use crate::server::AppState;

/// Set each `(key, value)` on every pane in `block_ids` (a `null` value clears the key) and tell the
/// windows. One pane failing doesn't stop the rest.
pub(crate) fn set_block_meta(state: &AppState, block_ids: &[&str], updates: &[(&str, Value)]) {
    let mut update = obj::MetaMapType::new();
    for (key, value) in updates {
        update.insert((*key).to_string(), value.clone());
    }
    for oid in block_ids {
        match crate::server::service::update_object_meta(&state.mstore, &format!("block:{oid}"), &update) {
            Ok(()) => blockcontroller::core::broadcast_block_update(&state.mstore, &state.event_bus, oid),
            Err(e) => tracing::warn!(block_id = %oid, error = %e, "prep: the pane's meta was not updated"),
        }
    }
}

/// `agent:prep` for a window that is open until `until_ms`.
pub(super) fn prep_value(id: &str, action: &str, until_ms: u64, extended: bool, why: Option<&str>) -> Value {
    json!({ "id": id, "action": action, "until_ms": until_ms, "extended": extended, "why": why })
}
