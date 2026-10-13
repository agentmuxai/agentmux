// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The subagent watcher's events, as typed payloads (`mps_events`). Each serializes exactly the
//! camelCase JSON the watcher built by hand before.

use serde::Serialize;

use super::types::{DispatchKind, DispatchStatus, SubagentEvent};

/// `subagent:activity`: new events from one subagent.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SubagentActivityEvent {
    pub agent_id: String,
    pub parent_agent: String,
    pub parent_block_id: String,
    #[ts(type = "number")]
    pub new_events: usize,
    #[ts(type = "number")]
    pub total_events: usize,
    pub events: Vec<SubagentEvent>,
    pub dispatch_id: String,
}

/// One member's new events in a `dispatch:activity` flush.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct DispatchMemberEvents {
    pub agent_id: String,
    pub events: Vec<SubagentEvent>,
}

/// `dispatch:activity`: a workflow dispatch's coalesced new events.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct DispatchActivityEvent {
    pub dispatch_id: String,
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    #[ts(type = "number")]
    pub new_events: usize,
    pub members: Vec<DispatchMemberEvents>,
}

/// `subagent:spawned`.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SubagentSpawnedEvent {
    pub agent_id: String,
    pub slug: String,
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    /// Null when the transcript doesn't say.
    pub model: Option<String>,
    pub dispatch_id: String,
}

/// `subagent:completed`. Completion found by a scan doesn't know the parent agent's name.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SubagentCompletedEvent {
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parent_agent: Option<String>,
    pub parent_block_id: String,
    #[ts(type = "number")]
    pub total_events: usize,
    pub dispatch_id: String,
}

/// `subagent:named`: a subagent got its display name.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SubagentNamedEvent {
    pub agent_id: String,
    pub display_name: String,
}

/// `subagent:updated`: something about a subagent changed; refetch it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SubagentUpdatedEvent {
    pub agent_id: String,
    pub parent_block_id: String,
}

/// `dispatch:updated`: a dispatch's membership or status.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct DispatchUpdatedEvent {
    pub dispatch_id: String,
    pub kind: DispatchKind,
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    #[ts(type = "number")]
    pub member_count: usize,
    #[ts(type = "number")]
    pub members_done: usize,
    pub status: DispatchStatus,
    pub dispatch_name: Option<String>,
}

/// `subagent:abandoned`: subagents of a pane cut off without finishing.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SubagentAbandonedEvent {
    pub parent_block_id: String,
    pub agent_ids: Vec<String>,
}

/// `subagent:block_pruned`: a pane's subagents are gone with the pane.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SubagentBlockPrunedEvent {
    pub block_id: String,
}

/// `subagent:backfill_status`: a resumed pane's subagent backfill started or finished.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct SubagentBackfillStatusEvent {
    #[ts(type = r#""started" | "done""#)]
    pub status: String,
}
