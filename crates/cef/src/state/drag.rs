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

/// Phase 4b — ghost state stored per target-window label during a floating-pane
/// redock drag. The target renderer pushes `block_id + dir` when it shows the
/// ghost overlay; the floater reads it at drop time to pass a directional hint
/// to the `RedockFloatingPane` saga.
#[derive(Clone, Debug)]
pub struct FloatingRedockGhostState {
    pub block_id: String,
    pub dir: u8,
}
