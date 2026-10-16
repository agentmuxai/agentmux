// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Stop: one tool for the actions that end work, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.1 rule 6 and
//! §4.2, slice S4). Kept apart from the other tools so a harness can ask before
//! any of them. It replaces QuitSelf, FleetBulkStop and ClosePane; their
//! handlers (panes.rs, fleet.rs) run each action unchanged, and srv's own
//! guards (the user's override window, the audit log) apply to all three.

use super::family::{pick, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "self", tool: "QuitSelf", rename: &[] },
    Act { action: "agents", tool: "FleetBulkStop", rename: &[] },
    Act { action: "pane", tool: "ClosePane", rename: &[] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Stop" => {
            let (act, args) = pick("Stop", ACTS, arguments)?;
            // ClosePane with no block_id ended the caller's own session without
            // QuitSelf's quote check; here ending your own session is `self`.
            // (Closing a pane you are in, by id, waits for the user's window
            // in srv like any other pane.)
            if act.action == "pane" && str_arg(&args, "block_id").is_none() {
                anyhow::bail!("Stop pane needs block_id (from Pane layout); to end your own session, use action \"self\"");
            }
            // `self` reaches only the caller's own block: a target is a
            // mistake, refused rather than ignored.
            if act.action == "self" && (args.get("block_id").is_some() || args.get("targets").is_some()) {
                anyhow::bail!("Stop self ends only your own session and takes no block_id or targets");
            }
            let result = match act.tool {
                "FleetBulkStop" => fleet::call(act.tool, &args, cx).await,
                _ => panes::call(act.tool, &args, cx).await,
            };
            relabeled("Stop", ACTS, result)
        }
        _ => Err(not_in_family()),
    }
}

/// `key`'s value, if it is a non-empty string.
fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::STOP_TOOL).unwrap();
        let mut listed: Vec<&str> = schema["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        let mut want = super::super::family::action_names(ACTS);
        listed.sort();
        want.sort();
        assert_eq!(listed, want);
    }

    /// Closing a pane never stands in for ending your own session: without a
    /// block_id it is refused before anything is sent.
    #[tokio::test]
    async fn pane_without_a_target_is_refused() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        for args in [json!({ "action": "pane" }), json!({ "action": "pane", "block_id": " " })] {
            let e = call("Stop", &args, &cx).await.unwrap_err().to_string();
            assert!(e.starts_with("Stop pane needs block_id"), "{e}");
        }
        // And `self` can't be aimed at anyone else.
        for args in [
            json!({ "action": "self", "reason": "r", "user_instruction": "quit", "block_id": "b2" }),
            json!({ "action": "self", "reason": "r", "user_instruction": "quit", "targets": ["b2"] }),
        ] {
            let e = call("Stop", &args, &cx).await.unwrap_err().to_string();
            assert!(e.starts_with("Stop self ends only your own session"), "{e}");
        }
    }
}
