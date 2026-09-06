// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Did this subagent actually finish, or was it cut off?
//!
//! `SubAgentStatus::Abandoned` is — as its own doc comment says —  "always an
//! inference, never an observation". Until this module existed, the inference
//! was "the subagent transcript has no terminal `Result` line, therefore it was
//! interrupted" (`scan.rs::reconcile_stale_subagents`). That premise does not
//! hold: **no AgentMux transcript contains a `"type":"result"` line at all** —
//! not the subagent sidechain files, not the parent session file. The line type
//! the old check waited for is absent from the format this deployment writes
//! (`entrypoint: sdk-cli`), so `SubAgentStatus::Completed` was unreachable and
//! every Agent-tool dispatch ended up displayed as "interrupted", including the
//! ones that returned full results the parent went on to use.
//!
//! Evidence and the full chain:
//! `docs/reports/REPORT_SUBAGENT_COMPLETION_NEVER_DETECTED_2026_09_05.md`.
//!
//! The replacement uses the strongest evidence available, and it is an
//! *observation* rather than an inference: the parent's own `tool_result` for
//! the dispatch's `tool_use_id`. If the parent recorded a result, the subagent
//! demonstrably returned — whatever its transcript does or doesn't end with.
//! The correlation key is already on disk: Claude Code writes an
//! `agent-<id>.meta.json` sidecar next to each subagent transcript carrying the
//! parent-side `toolUseId`.
//!
//! Deliberately kept free of watcher state so the decision is unit-testable on
//! its own — the same reasoning that split `parse.rs` out.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use super::types::SubAgentStatus;

/// One subagent's `.meta.json` sidecar. Only the field we correlate on is
/// modelled; the sidecar also carries `agentType`/`description`/`spawnDepth`/
/// `model`, which this module has no use for. `serde` ignores the rest.
#[derive(Debug, serde::Deserialize)]
struct SubagentMeta {
    #[serde(rename = "toolUseId")]
    tool_use_id: Option<String>,
}

/// The parent-side `tool_use_id` for a subagent transcript, read from its
/// sidecar. `None` when there is no sidecar, it is unreadable, it is malformed,
/// or it predates the field — all of which must degrade to the old
/// conservative behaviour rather than erroring.
pub(super) fn tool_use_id_for(subagent_jsonl: &Path) -> Option<String> {
    let meta_path = sidecar_path(subagent_jsonl)?;
    let raw = std::fs::read_to_string(meta_path).ok()?;
    serde_json::from_str::<SubagentMeta>(&raw).ok()?.tool_use_id
}

/// `…/agent-<id>.jsonl` → `…/agent-<id>.meta.json`.
fn sidecar_path(subagent_jsonl: &Path) -> Option<PathBuf> {
    let stem = subagent_jsonl.file_stem()?.to_str()?;
    Some(subagent_jsonl.with_file_name(format!("{stem}.meta.json")))
}

/// The parent session transcript for a subagent transcript.
///
/// Claude Code lays these out as `…/<session-id>.jsonl` alongside a
/// `…/<session-id>/subagents/` tree, but members are NOT all at the same
/// depth inside it (`scan_subagents_dir`'s own doc comment):
///
/// ```text
/// <session-id>/subagents/agent-<id>.jsonl                      ← Task-tool (solo)
/// <session-id>/subagents/workflows/<run-id>/agent-<id>.jsonl   ← Workflow-tool member
/// ```
///
/// So this walks up to the `subagents` directory rather than assuming it is
/// the immediate parent. Checking only the immediate parent silently excluded
/// every Workflow-tool member — they could never resolve a completion and
/// stayed permanently "interrupted", i.e. exactly the bug this module exists
/// to fix, left unfixed for that dispatch kind (reagent P1 on #3007).
///
/// Returns `None` when there is no `subagents` ancestor at all, rather than
/// guessing at a path and reading some unrelated file as a transcript.
pub(super) fn parent_transcript_for(subagent_jsonl: &Path) -> Option<PathBuf> {
    let subagents_dir = subagent_jsonl
        .ancestors()
        .find(|a| a.file_name().and_then(|n| n.to_str()) == Some("subagents"))?;
    let session_dir = subagents_dir.parent()?;
    let session_id = session_dir.file_name()?.to_str()?;
    Some(session_dir.with_file_name(format!("{session_id}.jsonl")))
}

/// Every `tool_use_id` the parent transcript has recorded a `tool_result` for,
/// mapped to that result's `is_error` flag.
///
/// The caller reads once per DISTINCT parent transcript, not once per
/// subagent: a transcript is routinely tens of MB (26 MB / 15k lines on the
/// session that surfaced this) and a pass can hold many members, but they do
/// not all resolve to the same parent (see [`parent_transcript_for`]). The
/// cheap `contains` pre-filter keeps the JSON parse off the ~99% of lines that
/// carry no tool result at all.
pub(super) fn parent_tool_results(reader: impl BufRead) -> HashMap<String, bool> {
    let mut out = HashMap::new();
    for line in reader.lines().map_while(Result::ok) {
        if !line.contains("tool_use_id") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(content) = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                continue;
            }
            let Some(id) = block.get("tool_use_id").and_then(|v| v.as_str()) else {
                continue;
            };
            let is_error = block
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            out.insert(id.to_string(), is_error);
        }
    }
    out
}

/// Read an already-resolved parent transcript. An unreadable/absent file
/// yields an empty map, which makes [`terminal_status`] fall back to the old
/// `Abandoned` inference — conservative, and identical to pre-fix behaviour.
///
/// Takes the resolved path rather than a subagent path so the caller can
/// cache one read per distinct parent: members of a reconcile pass do not all
/// resolve to the same transcript (see [`parent_transcript_for`]), so
/// resolving once from an arbitrary member and reusing it for the rest is
/// wrong — that was reagent's P1 on #3007.
pub(super) fn parent_tool_results_at(parent_jsonl: &Path) -> HashMap<String, bool> {
    let Ok(file) = std::fs::File::open(parent_jsonl) else {
        return HashMap::new();
    };
    parent_tool_results(std::io::BufReader::new(file))
}

/// What an `Active` subagent's status should become once its parent turn is
/// confirmed idle.
///
/// `Completed` when the parent recorded a `tool_result` for this dispatch —
/// proof it returned. `Abandoned` otherwise, which now means what it always
/// claimed to: no evidence it ever came back.
///
/// **An errored result still counts as `Completed`.** The distinction this
/// draws is *returned* vs *cut off*, and a dispatch that reported an error
/// returned. Folding failures into `Abandoned` would recreate the same
/// conflation this fix exists to remove, just with a smaller blast radius.
/// Surfacing the error state itself needs a status the enum doesn't have yet
/// (and a matching frontend union) — tracked as follow-up in the report above;
/// `is_error` is logged at the call site meanwhile so it isn't silently lost.
pub(super) fn terminal_status(
    tool_use_id: Option<&str>,
    parent_results: &HashMap<String, bool>,
) -> SubAgentStatus {
    match tool_use_id {
        Some(id) if parent_results.contains_key(id) => SubAgentStatus::Completed,
        _ => SubAgentStatus::Abandoned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // Shapes below are taken from the real transcripts that surfaced this bug
    // (manoz session 468e2051…), not invented.

    #[test]
    fn parent_transcript_sits_beside_the_session_dir() {
        let sub = Path::new("/p/C--proj/468e2051-abc/subagents/agent-a5cb.jsonl");
        assert_eq!(
            parent_transcript_for(sub),
            Some(PathBuf::from("/p/C--proj/468e2051-abc.jsonl"))
        );
    }

    #[test]
    fn a_workflow_member_resolves_to_the_same_session_transcript() {
        // Workflow-tool members sit one level deeper than Task-tool ones
        // (subagents/workflows/<run-id>/). Requiring `subagents` to be the
        // IMMEDIATE parent excluded every one of them, so they could never
        // resolve a completion and stayed permanently "interrupted" —
        // reagent P1 on #3007. No fixture on the dev machine had a workflow
        // run, which is why the original tests missed it.
        let sub = Path::new("/p/C--proj/468e2051-abc/subagents/workflows/wf_9/agent-a5cb.jsonl");
        assert_eq!(
            parent_transcript_for(sub),
            Some(PathBuf::from("/p/C--proj/468e2051-abc.jsonl"))
        );
    }

    #[test]
    fn solo_and_workflow_members_of_one_session_agree_on_the_parent() {
        // The mixed-batch case: both must resolve to the SAME transcript, or
        // a pass containing both silently mis-resolves half of them.
        let solo = Path::new("/p/C--proj/sess-1/subagents/agent-solo.jsonl");
        let member = Path::new("/p/C--proj/sess-1/subagents/workflows/wf_1/agent-m.jsonl");
        assert_eq!(parent_transcript_for(solo), parent_transcript_for(member));
    }

    #[test]
    fn a_path_that_is_not_in_a_subagents_dir_is_not_guessed_at() {
        // Better to decline than to invent a parent path and read some
        // unrelated file as if it were the session transcript.
        let sub = Path::new("/p/C--proj/468e2051-abc/agent-a5cb.jsonl");
        assert_eq!(parent_transcript_for(sub), None);
    }

    #[test]
    fn sidecar_sits_beside_the_transcript() {
        assert_eq!(
            sidecar_path(Path::new("/x/subagents/agent-a5cb.jsonl")),
            Some(PathBuf::from("/x/subagents/agent-a5cb.meta.json"))
        );
    }

    #[test]
    fn collects_tool_result_ids_with_their_error_flag() {
        let lines = r#"
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_ok","is_error":false,"content":"done"}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_bad","is_error":true,"content":"boom"}]}}
"#;
        let got = parent_tool_results(Cursor::new(lines));
        assert_eq!(got.get("toolu_ok"), Some(&false));
        assert_eq!(got.get("toolu_bad"), Some(&true));
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn a_missing_is_error_flag_reads_as_success() {
        // Real results routinely omit it; absent must not read as failure.
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_x","content":"ok"}]}}"#;
        assert_eq!(parent_tool_results(Cursor::new(line)).get("toolu_x"), Some(&false));
    }

    #[test]
    fn ignores_tool_use_lines_and_other_noise() {
        // A `tool_use` line mentions no tool_use_id and must not be mistaken
        // for evidence that the call came BACK.
        let lines = r#"
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_x","name":"Agent","input":{}}]}}
{"type":"queue-operation","operation":"enqueue","content":"u there"}
not json at all
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}
"#;
        assert!(parent_tool_results(Cursor::new(lines)).is_empty());
    }

    #[test]
    fn a_returned_dispatch_is_completed() {
        let mut results = HashMap::new();
        results.insert("toolu_ok".to_string(), false);
        assert_eq!(
            terminal_status(Some("toolu_ok"), &results),
            SubAgentStatus::Completed
        );
    }

    #[test]
    fn an_errored_dispatch_still_returned_so_it_is_completed() {
        // Returned-vs-cut-off is the distinction; an error is a return.
        let mut results = HashMap::new();
        results.insert("toolu_bad".to_string(), true);
        assert_eq!(
            terminal_status(Some("toolu_bad"), &results),
            SubAgentStatus::Completed
        );
    }

    #[test]
    fn a_dispatch_with_no_parent_result_is_genuinely_abandoned() {
        // The case the old code assumed was universal — still handled.
        assert_eq!(
            terminal_status(Some("toolu_never_returned"), &HashMap::new()),
            SubAgentStatus::Abandoned
        );
    }

    #[test]
    fn an_unknown_tool_use_id_falls_back_to_the_old_inference() {
        // No sidecar / older layout: must degrade to prior behaviour rather
        // than optimistically completing something we cannot correlate.
        let mut results = HashMap::new();
        results.insert("toolu_ok".to_string(), false);
        assert_eq!(terminal_status(None, &results), SubAgentStatus::Abandoned);
    }
}
