// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for a host agent that shares a folder with a container agent (`folderoverlapcheck`,
//! `folderoverlapaccept`, `folderoverlapdecline`, `folderoverlapasks`).

use serde::{Deserialize, Serialize};

/// `folderoverlapcheck`: the container agents `agent_id` shares a folder with, that the person has not gone ahead
/// with. `folder` is the folder a host agent is about to open in, when it is not the one its definition names.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandFolderOverlapCheckData {
    pub agent_id: String,
    #[serde(default)]
    #[ts(optional)]
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FolderOverlapCheckResult {
    pub overlaps: Vec<crate::backend::folder_overlap::Overlap>,
}

/// `folderoverlapaccept`: the person goes ahead with this host agent and this container agent sharing a folder.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandFolderOverlapAcceptData {
    pub host_id: String,
    pub container_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FolderOverlapAcceptResult {
    pub chosen: bool,
}

/// `folderoverlapdecline`: the person says no to an agent's ask for the go-ahead with this pair. The pair stays asking.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct CommandFolderOverlapDeclineData {
    pub host_id: String,
    pub container_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FolderOverlapDeclineResult {
    pub declined: bool,
}

/// `folderoverlapasks`: the asks waiting for the person, for a window that starts after they were made.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FolderOverlapAsksResult {
    pub asks: Vec<crate::backend::folder_overlap_asks::FolderAsk>,
}
