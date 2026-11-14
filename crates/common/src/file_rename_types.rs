// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The request of `POST /api/v1/file/rename`, which agentmux-mcp's `Pane` (`rename` with target
//! `file`) sends to srv. Kept out of `api_types.rs`, which re-exports it, for that file's size.

use serde::{Deserialize, Serialize};

use crate::api_types::UiAutomationAuth;

/// `POST /api/v1/file/rename` — rename a file or folder in its folder. Editor tabs open on it follow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRenameRequest {
    /// The file or folder: absolute on this computer, the host's own on an SSH host, in
    /// `/workspace` for a container agent.
    pub path: String,
    /// The new name: a plain name, with no folder.
    pub name: String,
    /// The SSH host the file is on; absent for this computer. Needs `block_id` and `auth`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    /// The calling agent's pane, which shows the consent for an SSH host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    /// The calling agent's signed identity, for an SSH `connection`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<UiAutomationAuth>,
}
