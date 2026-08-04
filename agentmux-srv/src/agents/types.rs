// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unified agent types shared between the agent pane (interactive)
//! and the drone Agent block (headless). See
//! `docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md` §3 for the
//! full design rationale.
//!
//! Wire format is camelCase via `serde(rename_all)` so the TS
//! mirror in `frontend/types/gotypes.d.ts` requires no field
//! translation.

use serde::{Deserialize, Serialize};

/// Identifies "which agent." Empty-string sentinels match the
/// existing wstore `AgentInstance` conventions. All fields optional
/// so callers can construct anything from "blank claude with ambient
/// creds" (all empty) up to a fully-pinned named-agent continuation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    /// Legacy Identity-bundle id — `db_identity_bundles` was dropped in
    /// Phase 4c of SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md, so this
    /// field is now vestigial (never read at spawn; credential resolution
    /// is `db_agent_identity_links`-only). Empty = blank singleton
    /// (ambient creds, no env-var injection at spawn).
    #[serde(default)]
    pub identity_id: String,
    /// FK to `db_bundles.id`. Empty = blank singleton (vanilla CLI,
    /// no system instructions injected).
    #[serde(default)]
    pub memory_id: String,
    /// User-chosen instance name. Empty for one-shot launches.
    /// Non-empty triggers the named-agent continuation path: the
    /// runner looks up an existing `AgentInstance` by name and reuses
    /// its `working_directory` + `session_id` if present.
    #[serde(default)]
    pub instance_name: String,
    /// Optional explicit working directory override. Empty falls
    /// back to `allocate_agent_workdir()` at run time.
    #[serde(default)]
    pub working_directory: String,
}

/// What the agent should do, plus the variables for `{{ }}` resolution
/// inside `prompt`. The agent pane uses `prompt=<user-typed-text>`
/// with an empty `context`. The drone Agent block uses
/// `prompt=<block.data.task>` resolved against `scope.outputs +
/// scope.vars`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTask {
    pub prompt: String,
    /// Variable scope for template resolution inside `prompt`. Keys
    /// are typically block ids or `var`/`env` namespaces; values are
    /// JSON. The runner is responsible for resolution before spawn.
    #[serde(default)]
    pub context: serde_json::Map<String, serde_json::Value>,
    /// Hard cap on turns. `None` = use the provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

/// Discriminated streaming event. Same union for both the agent
/// pane (renders into the UI) and the drone Agent block
/// (accumulates until `Done`, returns `AgentRunResult`).
///
/// Provider-specific extension goes through a `Custom` variant
/// reserved here but intentionally not shipped Phase 1.5 — leave
/// the enum open for it. See spec §8 risks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum AgentEvent {
    /// Streaming text chunk from the assistant. Agent pane appends
    /// to the visible transcript; drone Agent block buffers
    /// until `Done`.
    AssistantText {
        delta: String,
    },
    /// Tool invocation about to run. `input` is the provider's raw
    /// tool input JSON; renderers may dispatch on `tool` name.
    ToolUse {
        tool_use_id: String,
        tool: String,
        input: serde_json::Value,
    },
    /// Tool execution result.
    ToolResult {
        tool_use_id: String,
        output: serde_json::Value,
        #[serde(default)]
        is_error: bool,
    },
    /// Final cost + token accounting. Emitted once per run, before
    /// `Done`.
    Cost {
        cost_usd: f64,
        tokens: TokenCounts,
    },
    /// Run completed successfully. `response` is the final assistant
    /// message text (the drone Agent block's primary output).
    /// `transcript` is the full ordered turn list for audit / replay.
    Done {
        response: String,
        transcript: Vec<AgentTurn>,
    },
    /// Run failed. `message` is the user-facing error.
    Error {
        message: String,
    },
    /// Context compaction completed. Sourced from Claude Code's
    /// `type: "system", subtype: "compact_boundary"` stream-json
    /// frame — exact data, not a heuristic. See
    /// `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
    /// §2/§4.1 for the full design rationale and the source frame
    /// shape this is translated from.
    CompactionBoundary {
        trigger: CompactionTrigger,
        pre_tokens: u64,
        post_tokens: u64,
        cumulative_dropped_tokens: u64,
        duration_ms: u64,
    },
}

/// How compaction was triggered. `Auto` = context filled up and
/// Claude Code compacted automatically; `Manual` = the user (or the
/// agent itself) ran `/compact`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub cache_creation: u64,
    #[serde(default)]
    pub cache_read: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurn {
    /// `"user"` | `"assistant"` | `"tool_result"`.
    pub role: String,
    pub content: serde_json::Value,
    pub timestamp_ms: i64,
}

/// Final structured result of a complete agent run — the value the
/// drone Agent block returns to downstream blocks. The agent
/// pane discards this (it has already rendered the stream) but
/// constructs the same struct for the audit log.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunResult {
    pub response: String,
    pub tokens: TokenCounts,
    pub cost_usd: f64,
    pub transcript: Vec<AgentTurn>,
    /// Terminal stream-json `result` frame when it reported an error
    /// (`is_error` / `error_*` subtype). Internal only — `#[serde(skip)]`
    /// keeps it off the IPC wire. Lets the runner fail a run that claude
    /// reported as an error on stdout while still exiting 0. (codex P1 #1353.)
    #[serde(skip)]
    pub error_frame: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ────────────────────────────────────────────────────────────────
    // Wire format — verify camelCase on the JSON side. The TS mirror
    // in `frontend/types/gotypes.d.ts` depends on this; any drift
    // becomes silent type errors at the IPC seam.
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn agent_ref_serializes_camelcase() {
        let r = AgentRef {
            identity_id: "id1".into(),
            memory_id: "mem1".into(),
            instance_name: "alice".into(),
            working_directory: "/tmp/x".into(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v,
            json!({
                "identityId": "id1",
                "memoryId": "mem1",
                "instanceName": "alice",
                "workingDirectory": "/tmp/x"
            })
        );
    }

    #[test]
    fn agent_ref_defaults_round_trip() {
        // Empty-string sentinels match the wstore convention so the
        // frontend can omit fields it doesn't set.
        let r: AgentRef = serde_json::from_value(json!({})).unwrap();
        assert_eq!(r, AgentRef::default());
    }

    #[test]
    fn agent_event_assistant_text_shape() {
        let ev = AgentEvent::AssistantText {
            delta: "hi".into(),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v, json!({ "type": "assistant_text", "delta": "hi" }));
    }

    #[test]
    fn agent_event_tool_use_camelcase_id() {
        let ev = AgentEvent::ToolUse {
            tool_use_id: "tu_42".into(),
            tool: "bash".into(),
            input: json!({ "cmd": "ls" }),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v,
            json!({
                "type": "tool_use",
                "toolUseId": "tu_42",
                "tool": "bash",
                "input": { "cmd": "ls" }
            })
        );
    }

    #[test]
    fn agent_event_cost_shape() {
        let ev = AgentEvent::Cost {
            cost_usd: 0.0123,
            tokens: TokenCounts {
                input: 100,
                output: 50,
                cache_creation: 0,
                cache_read: 200,
            },
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v,
            json!({
                "type": "cost",
                "costUsd": 0.0123,
                "tokens": {
                    "input": 100,
                    "output": 50,
                    "cacheCreation": 0,
                    "cacheRead": 200
                }
            })
        );
    }

    #[test]
    fn agent_event_roundtrips() {
        let original = AgentEvent::Done {
            response: "ok".into(),
            transcript: vec![AgentTurn {
                role: "assistant".into(),
                content: json!("hi"),
                timestamp_ms: 1_700_000_000_000,
            }],
        };
        let s = serde_json::to_string(&original).unwrap();
        let parsed: AgentEvent = serde_json::from_str(&s).unwrap();
        // Match the shape, not the exact equality (transcript Vec).
        match parsed {
            AgentEvent::Done {
                response,
                transcript,
            } => {
                assert_eq!(response, "ok");
                assert_eq!(transcript.len(), 1);
                assert_eq!(transcript[0].role, "assistant");
                assert_eq!(transcript[0].timestamp_ms, 1_700_000_000_000);
            }
            _ => panic!("expected Done variant"),
        }
    }

    #[test]
    fn agent_event_compaction_boundary_shape() {
        let ev = AgentEvent::CompactionBoundary {
            trigger: CompactionTrigger::Manual,
            pre_tokens: 783_887,
            post_tokens: 11_775,
            cumulative_dropped_tokens: 772_112,
            duration_ms: 231_606,
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v,
            json!({
                "type": "compaction_boundary",
                "trigger": "manual",
                "preTokens": 783_887,
                "postTokens": 11_775,
                "cumulativeDroppedTokens": 772_112,
                "durationMs": 231_606
            })
        );
    }

    #[test]
    fn compaction_trigger_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(CompactionTrigger::Auto).unwrap(),
            json!("auto")
        );
        assert_eq!(
            serde_json::to_value(CompactionTrigger::Manual).unwrap(),
            json!("manual")
        );
    }

    #[test]
    fn agent_run_result_shape() {
        let r = AgentRunResult {
            response: "hi".into(),
            tokens: TokenCounts::default(),
            cost_usd: 0.0,
            transcript: vec![],
            error_frame: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        // costUsd at the result level, tokens nested with camelCase.
        assert_eq!(
            v,
            json!({
                "response": "hi",
                "tokens": {
                    "input": 0,
                    "output": 0,
                    "cacheCreation": 0,
                    "cacheRead": 0
                },
                "costUsd": 0.0,
                "transcript": []
            })
        );
    }
}
