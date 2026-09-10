// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


//! Bundle command shapes (v7). A Bundle is a db_bundles row; the type names
//! here still carry the pre-rename "Memory" wording because they are wire-
//! adjacent (two generate frontend bindings) and rename in a later phase.



use serde::{Deserialize, Serialize};

// ---- v7 Bundle command shapes ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandGetMemoryData {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandDeleteMemoryData {
    pub id: String,
}

/// Response for `deletememory` and `deletesystemmemory` — both take the
/// same request (`CommandDeleteMemoryData`, above) and answer with the
/// same shape, so they share this response type too. Was an anonymous
/// `json!({"deleted": ..})` before this type existed to name it for the
/// RPC bindings generator.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct DeleteMemoryResult {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandReorderGlobalBrainData {
    /// Full ordered list of global bundle ids. Each id's `sort_order`
    /// becomes its position in this list.
    pub ids: Vec<String>,
}

