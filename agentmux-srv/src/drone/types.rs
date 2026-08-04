// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drone + run types. Mirrors the frontend shape so RPC payloads
//! flow through `serde_json::to_value` without manual mapping.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Block (node) kinds for Phase 1. Phase 2 adds Function, Loop, Parallel,
/// Router, Subdrone. Stored as `kind` field on `FlowNode.data`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Agent,
    Condition,
    Api,
    Response,
    Variables,
}

impl BlockKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agent" => Some(Self::Agent),
            "condition" => Some(Self::Condition),
            "api" => Some(Self::Api),
            "response" => Some(Self::Response),
            "variables" => Some(Self::Variables),
            _ => None,
        }
    }
}

/// Position-and-data shape of a node on the canvas. Mirrors xyflow's
/// `Node` — id, position, data, type are the fields the canvas reads.
/// Anything inside `data` is block-specific config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: String,
    /// xyflow position. Saved as-is.
    #[serde(default)]
    pub position: NodePosition,
    /// Block kind + per-kind config (`task`, `url`, `expr`, etc.).
    pub data: serde_json::Value,
    /// Optional xyflow node type — keeps the canvas configurable.
    #[serde(default, rename = "type", skip_serializing_if = "String::is_empty")]
    pub node_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

/// xyflow `Edge` — source/target ids, optional handle ids.
///
/// Wire format matches xyflow's TS shape (camelCase: `sourceHandle` /
/// `targetHandle`) so JSON roundtrips through the canvas + frontend
/// `DroneFlowEdge` type without field-name translation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_handle: Option<String>,
}

/// Top-level graph payload — what the canvas saves and the executor reads.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DroneGraph {
    #[serde(default)]
    pub nodes: Vec<FlowNode>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DroneViewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for DroneViewport {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, zoom: 1.0 }
    }
}

/// Wstore row shape. Matches `db_drone_definitions` schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DroneDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub graph: DroneGraph,
    #[serde(default)]
    pub viewport: DroneViewport,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Done,
    Failed,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// One row in `db_drone_runs`. Append-only history of executions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DroneRun {
    pub id: String,
    pub drone_id: String,
    pub status: String,
    pub started_at: i64,
    pub ended_at: i64,
    /// Map of block_id → BlockState snapshot at run completion.
    #[serde(default)]
    pub block_states: HashMap<String, BlockState>,
    /// Final output captured by the Response block (stringified JSON).
    #[serde(default)]
    pub output: String,
    /// Top-level error message if the run failed before reaching Response.
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockState {
    pub status: String, // "pending" | "running" | "done" | "error" | "skipped"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}
