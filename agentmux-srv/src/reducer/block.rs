// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{ErrorCode, Event};

use crate::state::State;


use crate::state::BlockRecord;

/// Phase E.3 — create a block inside a tab. Validates parent tab
/// exists; otherwise emits `Event::Error` (non-fatal). On success:
/// assigns a UUID, appends to the tab's `block_ids`, inserts into
/// `state.blocks`, emits `Event::BlockCreated`.
///
/// NOT idempotent on retry (UUID assignment per call); saga-side
/// dedup is responsible for at-most-once delivery in E.5+.
pub(super) fn handle_create_block(
    state: &mut State,
    tab_id: String,
    meta: serde_json::Value,
) -> Vec<Event> {
    if !state.tabs.contains_key(&tab_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("CreateBlock: tab not found: {}", tab_id),
            fatal: false,
            version: v,
        }];
    }
    let block_id = uuid::Uuid::new_v4().to_string();
    state.blocks.insert(
        block_id.clone(),
        BlockRecord {
            block_id: block_id.clone(),
            tab_id: tab_id.clone(),
        },
    );
    let tab = state.tabs.get_mut(&tab_id).expect("checked");
    tab.block_ids.push(block_id.clone());
    let v = state.bump_version();
    vec![Event::BlockCreated {
        tab_id,
        block_id,
        meta,
        version: v,
    }]
}

/// Phase E.3 — delete a block from a tab. Idempotent: deleting a
/// missing tab or missing block is a silent no-op.
pub(super) fn handle_delete_block(state: &mut State, tab_id: String, block_id: String) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        return Vec::new();
    };
    let Some(pos) = tab.block_ids.iter().position(|b| b == &block_id) else {
        return Vec::new();
    };
    tab.block_ids.remove(pos);
    state.blocks.remove(&block_id);
    let v = state.bump_version();
    vec![Event::BlockDeleted {
        tab_id,
        block_id,
        version: v,
    }]
}

/// Phase E.5.5 — move a block from `src_tab_id` to `dst_tab_id` at
/// `dst_index` (clamped). Updates `block.tab_id`. Cross-tab moves
/// AND intra-tab repositioning both go through this command (the
/// caller specifies the destination index regardless).
///
/// Errors when source / dest tab missing, block missing, or
/// `block.tab_id != src_tab_id`.
pub(super) fn handle_move_block(
    state: &mut State,
    block_id: String,
    src_tab_id: String,
    dst_tab_id: String,
    dst_index: u32,
) -> Vec<Event> {
    let validation_error: Option<String> = {
        if !state.tabs.contains_key(&src_tab_id) {
            Some(format!("MoveBlock: src tab not found: {}", src_tab_id))
        } else if !state.tabs.contains_key(&dst_tab_id) {
            Some(format!("MoveBlock: dst tab not found: {}", dst_tab_id))
        } else {
            match state.blocks.get(&block_id) {
                None => Some(format!("MoveBlock: block not found: {}", block_id)),
                Some(block) if block.tab_id != src_tab_id => Some(format!(
                    "MoveBlock: block {} belongs to tab {}, not {}",
                    block_id, block.tab_id, src_tab_id
                )),
                _ => None,
            }
        }
    };
    if let Some(message) = validation_error {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message,
            fatal: false,
            version: v,
        }];
    }

    // Special-case intra-tab move: remove and re-insert in the same
    // tab. The clamp is computed AFTER the removal so dst_index
    // refers to the post-removal list (matches the spec's "position
    // in dst.tab_ids AFTER insertion" semantics for cross-tab moves).
    let final_dst_index: u32 = if src_tab_id == dst_tab_id {
        let tab = state.tabs.get_mut(&src_tab_id).expect("checked");
        tab.block_ids.retain(|id| id != &block_id);
        let clamped = (dst_index as usize).min(tab.block_ids.len());
        tab.block_ids.insert(clamped, block_id.clone());
        clamped as u32
    } else {
        // Remove from src.
        state
            .tabs
            .get_mut(&src_tab_id)
            .expect("checked")
            .block_ids
            .retain(|id| id != &block_id);
        // Insert into dst.
        let dst = state.tabs.get_mut(&dst_tab_id).expect("checked");
        let clamped = (dst_index as usize).min(dst.block_ids.len());
        dst.block_ids.insert(clamped, block_id.clone());
        // Update parent.
        state.blocks.get_mut(&block_id).expect("checked").tab_id = dst_tab_id.clone();
        clamped as u32
    };

    let v = state.bump_version();
    vec![Event::BlockMoved {
        block_id,
        src_tab_id,
        dst_tab_id,
        dst_index: final_dst_index,
        version: v,
    }]
}

/// Phase E.5.3 — pass-through for block meta updates. Same shape.
pub(super) fn handle_update_block_meta(
    state: &mut State,
    block_id: String,
    meta_patch: serde_json::Value,
) -> Vec<Event> {
    if !state.blocks.contains_key(&block_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("UpdateBlockMeta: block not found: {}", block_id),
            fatal: false,
            version: v,
        }];
    }
    let v = state.bump_version();
    vec![Event::BlockMetaUpdated {
        block_id,
        meta_patch,
        version: v,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::test_support::*;
    use crate::reducer::update;
    use agentmux_common::ipc::Command;

    #[test]
    fn create_block_validates_tab_exists() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateBlock { tab_id: "no-such-tab".into(), meta: serde_json::Value::Null },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        assert!(state.blocks.is_empty());
    }

    #[test]
    fn create_block_appends_to_tab_block_ids() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let events = update(
            &mut state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::BlockCreated { .. }));
        let block_id = match &events[0] {
            Event::BlockCreated { block_id, .. } => block_id.clone(),
            _ => panic!(),
        };
        assert_eq!(state.tabs[&tab_id].block_ids, vec![block_id.clone()]);
        assert_eq!(state.blocks[&block_id].tab_id, tab_id);
    }

    #[test]
    fn delete_block_removes_from_state_and_tab() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
            &ctx(2),
        );
        let block_id = state.tabs[&tab_id].block_ids[0].clone();
        let events = update(
            &mut state,
            Command::DeleteBlock {
                tab_id: tab_id.clone(),
                block_id: block_id.clone(),
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::BlockDeleted { .. }));
        assert!(!state.blocks.contains_key(&block_id));
        assert!(state.tabs[&tab_id].block_ids.is_empty());
    }

    #[test]
    fn delete_block_unknown_silent_no_op() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let events = update(
            &mut state,
            Command::DeleteBlock {
                tab_id,
                block_id: "ghost".into(),
            },
            &ctx(2),
        );
        assert!(events.is_empty());
    }

    // ---- Phase E.5.5 — MoveBlock tests ----

    #[test]
    fn move_block_cross_tab_updates_lists_and_parent() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let src_tab = create_tab(&mut state, &ws, "src");
        let dst_tab = create_tab(&mut state, &ws, "dst");
        let block = create_block(&mut state, &src_tab);
        let dst_existing = create_block(&mut state, &dst_tab);
        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: block.clone(),
                src_tab_id: src_tab.clone(),
                dst_tab_id: dst_tab.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::BlockMoved { .. }));
        assert_eq!(state.tabs[&src_tab].block_ids, Vec::<String>::new());
        assert_eq!(state.tabs[&dst_tab].block_ids, vec![block.clone(), dst_existing]);
        assert_eq!(state.blocks[&block].tab_id, dst_tab);
    }

    #[test]
    fn move_block_intra_tab_repositions() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let tab = create_tab(&mut state, &ws, "t");
        let b1 = create_block(&mut state, &tab);
        let b2 = create_block(&mut state, &tab);
        let b3 = create_block(&mut state, &tab);
        // Move b1 to position 2 (end after removal).
        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: b1.clone(),
                src_tab_id: tab.clone(),
                dst_tab_id: tab.clone(),
                dst_index: 2,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::BlockMoved { dst_index, .. } => assert_eq!(*dst_index, 2),
            other => panic!("expected BlockMoved, got {:?}", other),
        }
        assert_eq!(state.tabs[&tab].block_ids, vec![b2, b3, b1]);
    }

    #[test]
    fn move_block_rejects_unknown_src_or_dst_or_block() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let tab = create_tab(&mut state, &ws, "t");
        let other_tab = create_tab(&mut state, &ws, "other");
        let block = create_block(&mut state, &tab);

        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: block.clone(),
                src_tab_id: "ghost-src".into(),
                dst_tab_id: other_tab.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));

        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: block.clone(),
                src_tab_id: tab.clone(),
                dst_tab_id: "ghost-dst".into(),
                dst_index: 0,
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::Error { .. }));

        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: "ghost-block".into(),
                src_tab_id: tab,
                dst_tab_id: other_tab,
                dst_index: 0,
            },
            &ctx(4),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn move_block_rejects_when_block_belongs_to_different_tab() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let real_src = create_tab(&mut state, &ws, "real");
        let other = create_tab(&mut state, &ws, "other");
        let dst = create_tab(&mut state, &ws, "dst");
        let block = create_block(&mut state, &real_src);
        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: block,
                src_tab_id: other,
                dst_tab_id: dst,
                dst_index: 0,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::Error { message, .. } => {
                assert!(message.contains("belongs to tab"), "got: {}", message);
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }
}
