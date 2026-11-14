// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The widget install command's request (`server/widget_handlers.rs` re-exports it under its old path).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetInstallReq {
    /// A package folder, its `widget.json`, or a `.zip`.
    pub path: String,
    /// Replace an installed package with the same id.
    #[serde(default)]
    pub replace: bool,
}
