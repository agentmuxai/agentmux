// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drone: list, read and fire the drones on this computer, and read their runs
//! (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §6.5). srv holds the drones and decides what an agent
//! may fire; this only carries the call, as the agent its token names.

use super::*;

const ACTIONS: &[&str] = &["list", "get", "fire", "runs"];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Drone" => {
            require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
            let action = arguments.get("action").and_then(Value::as_str).map(str::trim).unwrap_or("");
            if !ACTIONS.contains(&action) {
                anyhow::bail!("Drone: unknown action {action:?}; use {}", ACTIONS.join(", "));
            }
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty()).ok_or_else(|| anyhow::anyhow!("AGENTMUX_AGENT_ID is not set"))?;
            let url = format!("{}/api/v1/drone/{action}", cx.local_url.trim_end_matches('/'));
            let reply = crate::srv_http::srv_post_json(cx.client, &url, cx.auth_key, &body_of(&self_id, arguments), &format!("Drone {action}")).await?;
            Ok(serde_json::to_string_pretty(&reply).unwrap_or_default())
        }
        _ => Err(not_in_family()),
    }
}

/// What srv is sent: the caller, and the arguments the actions take.
fn body_of(self_id: &str, arguments: &Value) -> Value {
    let mut body = json!({ "agent_id": self_id });
    for key in ["drone_id", "input", "limit"] {
        if let Some(v) = arguments.get(key).filter(|v| !v.is_null()) {
            body[key] = v.clone();
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_in_the_schema() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::DRONE_TOOL).unwrap();
        let mut listed: Vec<&str> = schema["inputSchema"]["properties"]["action"]["enum"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        let mut want = ACTIONS.to_vec();
        listed.sort();
        want.sort();
        assert_eq!(listed, want);
    }

    #[test]
    fn the_caller_is_the_agent_and_only_the_known_arguments_go_to_srv() {
        let args = json!({ "action": "fire", "drone_id": "d1", "input": { "n": 1 }, "limit": 5, "agent_id": "someone-else", "extra": 1 });
        let body = body_of("loap", &args);
        assert_eq!(body["agent_id"], "loap", "an argument can't name another caller");
        assert_eq!((body["drone_id"].as_str(), body["input"]["n"].as_i64(), body["limit"].as_i64()), (Some("d1"), Some(1), Some(5)));
        assert!(body.get("extra").is_none() && body.get("action").is_none());
    }

    #[tokio::test]
    async fn an_unknown_action_is_refused_before_anything_is_sent() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "http://127.0.0.1:1", auth_key: "k", block_id: "b", client: &client, loops: &loops, loop_counter: &counter };
        let e = call("Drone", &json!({ "action": "delete" }), &cx).await.unwrap_err().to_string();
        assert!(e.starts_with("Drone: unknown action"), "{e}");
    }
}
