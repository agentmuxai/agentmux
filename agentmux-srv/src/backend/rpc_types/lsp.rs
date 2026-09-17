// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the editor pane's three LSP commands.
//!
//! These lived as function-local `Cmd` structs inside each handler, which is
//! why the frontend restated all three shapes by hand: a type nested in a
//! closure cannot be named, let alone generated from. Spec:
//! `docs/specs/SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md`.

use serde::{Deserialize, Serialize};

/// Request for `lspstart`. The backend resolves the workspace root from
/// `file_path` rather than trusting the client to send one.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct LspStartReq {
    pub language: String,
    pub file_path: String,
}

/// Response for `lspstart`. Was an inline `json!({ "server_id": .., ..})`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct LspStartResult {
    pub server_id: String,
    /// The root the supervisor actually attached to, which is not necessarily
    /// derived from the `file_path` the caller sent — an already-running
    /// server for that language is reused.
    pub workspace_root: String,
}

/// Request for `lspsend`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct LspSendReq {
    pub server_id: String,
    /// An arbitrary LSP JSON-RPC message. The backend is a dumb proxy here —
    /// it re-serializes this and writes it to the server's stdin without
    /// inspecting it — so `unknown` is the honest TS type, and the
    /// hand-written stub already said so. `serde_json::Value` would generate
    /// as the crate's JSON alias, which is wider and less accurate about the
    /// fact that nothing may be assumed about the contents.
    #[ts(type = "unknown")]
    pub message: serde_json::Value,
}

/// Request for `lspstop`. Refcount-decrement; the server exits at zero.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct LspStopReq {
    pub server_id: String,
}
