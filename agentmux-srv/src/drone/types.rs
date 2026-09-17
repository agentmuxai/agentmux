// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drone + run types. Mirrors the frontend shape so RPC payloads
//! flow through `serde_json::to_value` without manual mapping.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Block (node) kinds for Phase 1. Phase 2 adds Function, Loop, Parallel,
/// Router, Subdrone. Stored as `kind` field on `FlowNode.data`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
// The TS name the frontend has always used. Renaming here rather than renaming
// the Rust type keeps every existing consumer untouched while the declaration
// becomes generated.
#[ts(rename = "DroneFlowNode")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct FlowNode {
    pub id: String,
    /// xyflow position. Saved as-is.
    #[serde(default)]
    pub position: NodePosition,
    /// Block kind + per-kind config (`task`, `url`, `expr`, etc.).
    ///
    /// `serde_json::Value` says nothing about the one field every block
    /// actually has, and the hand-written declaration did: keep the narrower
    /// TS. A `Value` here is not a promise that the contents are arbitrary,
    /// it is the absence of a per-kind Rust enum (Phase 2).
    #[ts(type = "Record<string, unknown> & { kind: string }")]
    pub data: serde_json::Value,
    /// Optional xyflow node type — keeps the canvas configurable.
    ///
    /// `Option<String>`, not `String`: the server omits the key entirely
    /// when it is blank, so a required `type: string` in the generated TS
    /// would be wrong in the read direction -- which is the direction the
    /// canvas uses. `is_blank` rather than `Option::is_none` keeps the
    /// pre-existing wire behaviour exactly, where an empty string is also
    /// omitted.
    #[serde(default, rename = "type", skip_serializing_if = "is_blank")]
    // ts-rs does not read `serde(rename)` -- without this the generated key is
    // `node_type`, which is not what goes on the wire and not what the canvas
    // reads. `flow_node_type_field_is_named_type_on_the_wire` pins it.
    #[ts(optional, rename = "type")]
    pub node_type: Option<String>,
}

fn is_blank(v: &Option<String>) -> bool {
    v.as_deref().unwrap_or("").is_empty()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(rename = "DroneNodePosition")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

/// xyflow `Edge` — source/target ids, optional handle ids.
///
/// Wire format matches xyflow's TS shape (camelCase: `sourceHandle` /
/// `targetHandle`) so JSON roundtrips through the canvas + frontend
/// `DroneFlowEdge` type without field-name translation.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "DroneFlowEdge")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct FlowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target_handle: Option<String>,
}

/// Top-level graph payload — what the canvas saves and the executor reads.
#[derive(Debug, Clone, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct DroneGraph {
    #[serde(default)]
    pub nodes: Vec<FlowNode>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct DroneDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub graph: DroneGraph,
    #[serde(default)]
    pub viewport: DroneViewport,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
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
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct DroneRun {
    pub id: String,
    pub drone_id: String,
    /// Deliberately not the `RunStatus` union: rows written by older builds
    /// are read back through this field, so it has to stay open.
    pub status: String,
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number")]
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

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(rename = "DroneBlockState")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct BlockState {
    /// A `String` in Rust so a row written by a newer build still reads back,
    /// but the closed set on the TS side is real and the frontend branches on
    /// it -- a bare `string` here would be a downgrade. Keep the union and let
    /// `block_status_union_covers_every_value_the_executor_writes` fail if the
    /// executor ever starts writing something outside it.
    #[ts(type = "\"pending\" | \"running\" | \"done\" | \"error\" | \"skipped\"")]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub completed_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire key is `type`, not `node_type`.
    ///
    /// ts-rs does not read `serde(rename)`, so the generated binding needs its
    /// own `#[ts(rename)]` — and nothing else would catch the two drifting
    /// apart, because both halves compile fine and the canvas just stops
    /// finding the field. Same failure as the identity `oauth_config_dir` tag.
    #[test]
    fn flow_node_type_field_is_named_type_on_the_wire() {
        let node = FlowNode {
            id: "n1".into(),
            position: NodePosition::default(),
            data: serde_json::json!({ "kind": "agent" }),
            node_type: Some("agent".into()),
        };
        let v = serde_json::to_value(&node).unwrap();
        assert_eq!(v["type"], "agent");
        assert!(
            v.get("node_type").is_none(),
            "the Rust field name must never reach the wire, got {v}",
        );

        // And the generated TS says the same thing.
        let ts = <FlowNode as ts_rs::TS>::inline();
        assert!(ts.contains("type?: string"), "generated TS lost the rename: {ts}");
        assert!(!ts.contains("node_type"), "generated TS kept the Rust name: {ts}");
    }

    /// A blank node type is omitted rather than written as `""`, which is what
    /// made `type` optional in TS in the first place. `is_blank` (not
    /// `Option::is_none`) is what keeps `Some("")` omitted too, so that
    /// switching the field to `Option` changed no bytes on the wire.
    #[test]
    fn a_blank_node_type_is_omitted_from_the_wire() {
        for blank in [None, Some(String::new())] {
            let node = FlowNode {
                id: "n1".into(),
                position: NodePosition::default(),
                data: serde_json::json!({ "kind": "agent" }),
                node_type: blank.clone(),
            };
            let v = serde_json::to_value(&node).unwrap();
            assert!(v.get("type").is_none(), "{blank:?} should be omitted, got {v}");
        }
    }

    /// The generated `DroneBlockState.status` is a closed union, which is only
    /// honest as long as the executor writes nothing outside it. It is a
    /// `String` in Rust — so if someone adds a sixth status, the compiler will
    /// not notice and the frontend's exhaustive branches will silently miss it.
    /// This is the thing that notices.
    #[test]
    fn block_status_union_covers_every_value_the_executor_writes() {
        let ts = <BlockState as ts_rs::TS>::inline();
        // Keep in sync with the status literals in
        // drone/executor/engine.rs. "pending" is in the union but never
        // written by the server; the frontend uses it for blocks it has not
        // heard about yet, which is why the union is wider than this list.
        for written in ["running", "done", "error", "skipped"] {
            assert!(
                ts.contains(&format!("\"{written}\"")),
                "the executor writes {written:?} but the generated union omits it: {ts}",
            );
        }
    }

    /// `BlockKind` and `RunStatus` are snake_case on the wire, and ts-rs
    /// derives that from `serde(rename_all)` by itself -- no `ts(rename_all)`
    /// needed. This test is what proves that, and is why the redundant
    /// attribute could be removed: it asserts the generated union against what
    /// serde actually writes rather than against a second attribute that was
    /// only ever agreeing with the first.
    #[test]
    fn enum_variants_are_snake_case_in_both_serde_and_the_generated_union() {
        let kind_ts = <BlockKind as ts_rs::TS>::inline();
        for k in [
            BlockKind::Agent,
            BlockKind::Condition,
            BlockKind::Api,
            BlockKind::Response,
            BlockKind::Variables,
        ] {
            let wire = serde_json::to_value(k).unwrap();
            let wire = wire.as_str().unwrap();
            assert_eq!(wire, wire.to_lowercase(), "{k:?} is not snake_case on the wire");
            assert!(
                kind_ts.contains(&format!("\"{wire}\"")),
                "generated BlockKind is missing {wire:?}: {kind_ts}",
            );
        }

        let run_ts = <RunStatus as ts_rs::TS>::inline();
        for r in [RunStatus::Running, RunStatus::Done, RunStatus::Failed] {
            let wire = serde_json::to_value(r).unwrap();
            let wire = wire.as_str().unwrap();
            assert_eq!(wire, r.as_str(), "serde and as_str disagree for {r:?}");
            assert!(
                run_ts.contains(&format!("\"{wire}\"")),
                "generated RunStatus is missing {wire:?}: {run_ts}",
            );
        }
    }

    /// 64-bit timestamps must generate as `number`, not `bigint`: the frontend
    /// does arithmetic on them and `JSON.parse` never produces a `bigint`.
    #[test]
    fn timestamps_generate_as_number() {
        for ts in [
            <DroneDefinition as ts_rs::TS>::inline(),
            <DroneRun as ts_rs::TS>::inline(),
            <BlockState as ts_rs::TS>::inline(),
        ] {
            assert!(!ts.contains("bigint"), "a 64-bit field leaked as bigint: {ts}");
        }
    }
}
