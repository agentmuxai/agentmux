// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Payloads of the broker events `agentmux-bashwrap` publishes through srv's
//! `POST /agentmux/wps/publish`. srv declares the same types for these events
//! (`backend::mps_events`), refuses a publish whose `data` isn't one of them, and
//! generates them for the frontend, so the three can't drift
//! (docs/specs/SPEC_CODEGEN_REMAINING_GAPS_2026_10_13.md, step 2).
//!
//! The TypeScript derives are behind the `ts` feature, which only srv turns on.

use serde::{Deserialize, Serialize};

/// The `tool_chunk` event, published to `block:<id>` while a Bash call runs. The call's
/// tool_use_id rides in the payload, not the event name, so the pane opens one subscription per
/// block and routes by `tool_id` (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export_to = "../../../frontend/types/rpc/"))]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum ToolChunk {
    /// A line of the call's output.
    Chunk {
        tool_id: String,
        kind: ToolChunkKind,
        content: String,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        timestamp: u64,
    },
    /// The call exited.
    Terminal {
        tool_id: String,
        exit_code: i32,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        timestamp: u64,
    },
    /// The wrapper's own pid, once, for a declared-background call
    /// (SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md).
    Pid {
        tool_id: String,
        pid: u32,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        timestamp: u64,
    },
    /// Which call the wrapper runs, for Tower's "started by". srv keeps it and doesn't forward it
    /// (`backend::tool_calls`).
    Call {
        tool_id: String,
        pid: u32,
        description: String,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        timestamp: u64,
    },
}

/// Where a `tool_chunk` line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export_to = "../../../frontend/types/rpc/"))]
#[serde(rename_all = "lowercase")]
pub enum ToolChunkKind {
    Stdout,
    Stderr,
    /// A line the wrapper writes itself (a timeout, a kill).
    System,
}

/// The `compaction_started` event: the `PreCompact` hook fired, so Claude Code is compacting.
/// camelCase on the wire, like srv's `AgentEvent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export_to = "../../../frontend/types/rpc/"))]
#[serde(rename_all = "camelCase")]
pub struct CompactionStarted {
    pub trigger: CompactionTrigger,
    pub session_id: String,
    /// RFC 3339.
    pub started_at: String,
}

/// How compaction was triggered. `Auto` = context filled up and
/// Claude Code compacted automatically; `Manual` = the user (or the
/// agent itself) ran `/compact`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export_to = "../../../frontend/types/rpc/"))]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    Auto,
    Manual,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_chunks_serialize_as_the_wrapper_always_sent_them() {
        let chunk = ToolChunk::Chunk { tool_id: "toolu_1".into(), kind: ToolChunkKind::Stderr, content: "x\n".into(), timestamp: 5 };
        assert_eq!(serde_json::to_value(&chunk).unwrap(), json!({"op": "chunk", "tool_id": "toolu_1", "kind": "stderr", "content": "x\n", "timestamp": 5}));
        let terminal = ToolChunk::Terminal { tool_id: "toolu_1".into(), exit_code: -1, timestamp: 6 };
        assert_eq!(serde_json::to_value(&terminal).unwrap(), json!({"op": "terminal", "tool_id": "toolu_1", "exit_code": -1, "timestamp": 6}));
        let pid = ToolChunk::Pid { tool_id: "toolu_1".into(), pid: 42, timestamp: 7 };
        assert_eq!(serde_json::to_value(&pid).unwrap(), json!({"op": "pid", "tool_id": "toolu_1", "pid": 42, "timestamp": 7}));
        let call = ToolChunk::Call { tool_id: "toolu_1".into(), pid: 42, description: "Run it".into(), timestamp: 8 };
        assert_eq!(serde_json::to_value(&call).unwrap(), json!({"op": "call", "tool_id": "toolu_1", "pid": 42, "description": "Run it", "timestamp": 8}));
        for v in [chunk, terminal, pid, call] {
            assert_eq!(serde_json::from_value::<ToolChunk>(serde_json::to_value(&v).unwrap()).unwrap(), v);
        }
    }

    #[test]
    fn compaction_started_serializes_camel_case() {
        let p = CompactionStarted { trigger: CompactionTrigger::Auto, session_id: "s1".into(), started_at: "2026-10-13T00:00:00Z".into() };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"trigger": "auto", "sessionId": "s1", "startedAt": "2026-10-13T00:00:00Z"}));
    }

    #[test]
    fn an_unknown_op_or_kind_is_refused() {
        assert!(serde_json::from_value::<ToolChunk>(json!({"op": "nope", "tool_id": "t"})).is_err());
        assert!(serde_json::from_value::<ToolChunk>(json!({"op": "chunk", "tool_id": "t", "kind": "tty", "content": "", "timestamp": 0})).is_err());
        assert!(serde_json::from_value::<CompactionStarted>(json!({"trigger": "sometimes", "sessionId": "", "startedAt": ""})).is_err());
    }
}
