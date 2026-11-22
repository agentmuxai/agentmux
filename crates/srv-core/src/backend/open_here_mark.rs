// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A pane bound with "Open here" and the refusal it is expected to get.
//!
//! The agent picker's Open here binds a fresh pane to an agent that is open in another pane. The
//! single-live-instance guard refuses that second pane (`open_in_other_pane`), and the pane answers
//! the refusal at once with Move here. The refusal is a step of the move, not news: written into the
//! pane's history it would stay there as a red line above the conversation after the agent has moved.
//! The window marks the pane (`agent:open_here`) before it binds; the refusal still reaches the pane's
//! recovery row, only the line in the history is left out.

use crate::backend::obj::{meta_get_bool, MetaMapType};
use crate::backend::storage::store::Store;

/// Block meta the window sets on a pane bound with Open here. srv clears it once the agent has moved.
pub const META_OPEN_HERE: &str = "agent:open_here";

/// Whether `message`, a refusal about to be written into the pane's history, is the one an Open here
/// pane expects: the agent is open in another pane, and this pane is marked.
pub fn is_expected_open_here_refusal(meta: &MetaMapType, message: &str) -> bool {
    meta_get_bool(meta, META_OPEN_HERE, false) && crate::backend::blockcontroller::persistent::is_held_elsewhere_error(message)
}

/// [`is_expected_open_here_refusal`] for the block `block_id` as stored.
pub fn expected_for_block(store: &Store, block_id: &str, message: &str) -> bool {
    store
        .must_get::<crate::backend::obj::Block>(block_id)
        .is_ok_and(|block| is_expected_open_here_refusal(&block.meta, message))
}

/// The agent has moved into this pane. Drop the mark, so every refusal after this one is news again,
/// and the "open in another pane" failure that asked for the move: it is no longer true, and left in
/// the pane's meta it brings the recovery row back when the pane reloads.
pub fn settle_after_move(block_id: &str, store: &std::sync::Arc<Store>, event_bus: &Option<std::sync::Arc<crate::backend::eventbus::EventBus>>) {
    let written = store.with_tx(|tx| {
        let mut block = tx.must_get::<crate::backend::obj::Block>(block_id)?;
        let mut update = MetaMapType::new();
        if block.meta.contains_key(META_OPEN_HERE) {
            update.insert(META_OPEN_HERE.to_string(), serde_json::Value::Null);
        }
        let refused = block
            .meta
            .get(crate::backend::blockcontroller::core::META_LAST_FAILURE)
            .and_then(|f| f.get("code"))
            .and_then(|c| c.as_str())
            == Some("open_in_other_pane");
        if refused {
            update.insert(crate::backend::blockcontroller::core::META_LAST_FAILURE.to_string(), serde_json::Value::Null);
        }
        if update.is_empty() {
            return Ok(false);
        }
        block.meta = crate::backend::obj::merge_meta(&block.meta, &update, true);
        tx.update(&mut block)?;
        Ok(true)
    });
    match written {
        Ok(true) => {
            if let Some(bus) = event_bus {
                crate::backend::blockcontroller::core::broadcast_block_update(store, bus, block_id);
            }
        }
        Ok(false) => {}
        Err(e) => tracing::warn!(block_id = %block_id, error = %e, "could not settle the pane after the move"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::blockcontroller::persistent::held_elsewhere_error;
    use serde_json::json;

    fn meta(pairs: &[(&str, serde_json::Value)]) -> MetaMapType {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn a_marked_pane_expects_the_refusal_for_an_agent_open_elsewhere() {
        let m = meta(&[(META_OPEN_HERE, json!(true))]);
        assert!(is_expected_open_here_refusal(&m, &held_elsewhere_error("blk-1", false)));
    }

    #[test]
    fn an_unmarked_pane_keeps_the_refusal_in_its_history() {
        assert!(!is_expected_open_here_refusal(&meta(&[]), &held_elsewhere_error("blk-1", false)));
        assert!(!is_expected_open_here_refusal(&meta(&[(META_OPEN_HERE, json!(false))]), &held_elsewhere_error("blk-1", false)));
    }

    #[test]
    fn a_marked_pane_keeps_every_other_refusal() {
        let m = meta(&[(META_OPEN_HERE, json!(true))]);
        assert!(!is_expected_open_here_refusal(&m, "Attachments aren't supported for container agents yet."));
        assert!(!is_expected_open_here_refusal(&m, "Agent was taken over by another AgentMux instance on this host."));
    }
}
