// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agents: one tool for the other agents (who is reachable, what they are
//! doing, opening them) and the caller's own history, with an `action`
//! (docs/specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md §4.2, slice S4).
//! It replaces DiscoverAgents, FleetList (the same call), ListConversations,
//! GetAgentTranscript, OpenAgent, SearchHistory and SupervisorNudge; their
//! handlers (fleet.rs, history.rs, panes.rs) run each action unchanged.

use super::family::{pick, public_params, relabeled, Act};
use super::*;

pub(super) const ACTS: &[Act] = &[
    Act { action: "list", tool: "DiscoverAgents", rename: &[] },
    Act { action: "conversations", tool: "ListConversations", rename: &[] },
    Act { action: "transcript", tool: "GetAgentTranscript", rename: &[] },
    Act { action: "open", tool: "OpenAgent", rename: &[("agent", "agent_id")] },
    Act { action: "search_history", tool: "SearchHistory", rename: &[] },
    // The handler's own `action` is the decision: `pick` takes Agents'
    // `action` out before it renames.
    Act { action: "nudge", tool: "SupervisorNudge", rename: &[("agent", "target_agent"), ("decision", "action")] },
];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Agents" => {
            let (act, args) = pick("Agents", ACTS, arguments)?;
            // Your own history only (see AGENTS_TOOL): another agent's goes
            // through the transcript_request rules, never through a search.
            if act.action == "search_history" && args.get("agent").is_some() {
                anyhow::bail!(
                    "Agents search_history searches only your own history and takes no agent; \
                     another agent's conversation is shared through its own visibility setting"
                );
            }
            let result = match act.tool {
                "DiscoverAgents" | "SupervisorNudge" => fleet::call(act.tool, &args, cx).await,
                "OpenAgent" => panes::call(act.tool, &args, cx).await,
                _ => history::call(act.tool, &args, cx).await,
            };
            let result = result.map_err(|e| anyhow::anyhow!(public_params(&format!("{e:#}"), act.rename)));
            relabeled("Agents", ACTS, result)
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::AGENTS_TOOL).unwrap();
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

    #[test]
    fn nudge_and_open_take_the_agent_as_their_handlers_name_it() {
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "nudge", "agent": "Korp", "decision": "decline", "reason": "done" })).unwrap();
        assert_eq!(act.tool, "SupervisorNudge");
        assert_eq!(args, json!({ "target_agent": "Korp", "action": "decline", "reason": "done" }));
        let (act, args) = pick("Agents", ACTS, &json!({ "action": "open", "agent": "Korp", "focus": false })).unwrap();
        assert_eq!(act.tool, "OpenAgent");
        assert_eq!(args, json!({ "agent_id": "Korp", "focus": false }));
        let (_, args) = pick("Agents", ACTS, &json!({ "action": "transcript", "agent": "Korp" })).unwrap();
        assert_eq!(args, json!({ "agent": "Korp" }));
    }

    /// The search reaches only the caller's own history: an agent is refused
    /// before anything is sent.
    #[tokio::test]
    async fn search_history_takes_no_agent() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        let e = call("Agents", &json!({ "action": "search_history", "query": "x", "agent": "Korp" }), &cx)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.starts_with("Agents search_history searches only your own history"), "{e}");
    }
}
