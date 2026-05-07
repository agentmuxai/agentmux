// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{Command, ErrorCode, Event};

use crate::state::State;

use super::Ctx;

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
