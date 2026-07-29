// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Cross-window drag types (ported from src-tauri/src/state.rs).

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DragType {
    Pane,
    Tab,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragSession {
    pub drag_id: String,
    pub drag_type: DragType,
    pub source_window: String,
    pub source_workspace_id: String,
    pub source_tab_id: String,
    pub payload: DragPayload,
    pub started_at: u64,
}

/// Native pointer-capture tab/pane tear-off (SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md).
/// A torn-off window being live-dragged by the frontend's pointer tracker:
/// resolved once at `engage_native_window_drag`, then `update_native_window_drag`
/// repositions it per pointermove via a single cached HWND + grab offset,
/// with no per-frame label→HWND re-resolution. Single active target — this
/// gesture has one cursor, matching the same assumption `floating_redock_ghost`
/// makes for the (unrelated) floating-pane redock ghost.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
pub struct NativeDragTarget {
    pub label: String,
    pub hwnd: isize,
    pub grab_offset_x: i32,
    pub grab_offset_y: i32,
}
