// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Sensitive: ask for an operation too sweeping for one agent's say-so, or, as
//! the super agent, decide such a request (docs/specs/SPEC_SUPER_AGENT_2026_10_25.md
//! §4). srv holds the requests, the checks and the countdown the person can veto;
//! this only carries the call, as the agent its token names.

use super::*;

const ACTIONS: &[&str] = &["request", "approve", "deny", "cancel", "status", "list"];

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Sensitive" => {
            require_agent_env(cx.local_url, cx.auth_key, cx.block_id)?;
            let action = arguments.get("action").and_then(Value::as_str).map(str::trim).unwrap_or("");
            if !ACTIONS.contains(&action) {
                anyhow::bail!("Sensitive: unknown action {action:?}; use {}", ACTIONS.join(", "));
            }
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty()).ok_or_else(|| anyhow::anyhow!("AGENTMUX_AGENT_ID is not set"))?;
            let url = format!("{}/api/v1/sensitive/{action}", cx.local_url.trim_end_matches('/'));
            let reply = crate::srv_http::srv_post_json(cx.client, &url, cx.auth_key, &body_of(&self_id, arguments), &format!("Sensitive {action}")).await?;
            Ok(serde_json::to_string_pretty(&reply).unwrap_or_default())
        }
        _ => Err(not_in_family()),
    }
}

/// What srv is sent: the caller, and the arguments the actions take.
fn body_of(self_id: &str, arguments: &Value) -> Value {
    let mut body = json!({ "agent_id": self_id });
    for key in ["op", "id", "reason", "facts"] {
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
        let schema: Value = serde_json::from_str(crate::tool_schemas::SENSITIVE_TOOL).unwrap();
        let mut listed: Vec<&str> = schema["inputSchema"]["properties"]["action"]["enum"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
        let mut want = ACTIONS.to_vec();
        listed.sort();
        want.sort();
        assert_eq!(listed, want);
    }

    #[test]
    fn the_caller_is_the_agent_and_only_the_known_arguments_go_to_srv() {
        let args = json!({ "action": "request", "op": "upgrade", "facts": { "version": "1.0" }, "reason": "why", "agent_id": "someone-else", "extra": 1 });
        let body = body_of("parlo", &args);
        assert_eq!(body["agent_id"], "parlo", "an argument can't name another caller");
        assert_eq!((body["op"].as_str(), body["facts"]["version"].as_str(), body["reason"].as_str()), (Some("upgrade"), Some("1.0"), Some("why")));
        assert!(body.get("extra").is_none() && body.get("action").is_none());
    }

    #[tokio::test]
    async fn an_unknown_action_is_refused_before_anything_is_sent() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "http://127.0.0.1:1", auth_key: "k", block_id: "b", client: &client, loops: &loops, loop_counter: &counter };
        let e = call("Sensitive", &json!({ "action": "shred" }), &cx).await.unwrap_err().to_string();
        assert!(e.starts_with("Sensitive: unknown action"), "{e}");
    }
}
