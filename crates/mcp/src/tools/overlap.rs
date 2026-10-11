// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who else is working on the same thing
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.2), and saying
//! what you are working on (`ClaimWork`, `ReleaseWork`, §3.4).

use super::*;

/// The query srv's `/api/v1/work-facts/who` takes for these arguments, plus
/// who is asking (srv prefers the request's agent token to both).
fn who_query(arguments: &Value, agent: &str, block_id: &str) -> Vec<(&'static str, String)> {
    let mut q: Vec<(&'static str, String)> = Vec::new();
    for key in ["path", "repo", "branch", "query"] {
        if let Some(v) = arguments.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty()) {
            q.push((key, v.to_string()));
        }
    }
    if !agent.trim().is_empty() {
        q.push(("agent", agent.trim().to_string()));
    }
    if !block_id.trim().is_empty() {
        q.push(("block", block_id.trim().to_string()));
    }
    q
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "WhoIsWorkingOn" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            let agent = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
            let query = who_query(arguments, &agent, block_id);
            let asked_pr = arguments.get("pr").is_some_and(|v| !v.is_null());
            if asked_pr && !query.iter().any(|(k, _)| matches!(*k, "path" | "repo" | "branch" | "query")) {
                anyhow::bail!("pr is not supported yet: pass path, repo, branch or query");
            }
            let url = format!("{}/api/v1/work-facts/who", local_url.trim_end_matches('/'));
            let mut body = srv_get_json(client, &url, auth_key, &query, "WhoIsWorkingOn").await?;
            if asked_pr {
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("pr_note".into(), json!("pr is not supported yet and was ignored"));
                }
            }
            Ok(serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string()))
        }
        "ClaimWork" | "ReleaseWork" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            let agent = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
            let base = local_url.trim_end_matches('/');
            let (url, body) = if name == "ClaimWork" {
                (format!("{base}/api/v1/work-claims"), claim_body(arguments, &agent, block_id))
            } else {
                (format!("{base}/api/v1/work-claims/release"), release_body(arguments, &agent, block_id))
            };
            let answer = srv_post_json(client, &url, auth_key, &body, name).await?;
            Ok(serde_json::to_string_pretty(&answer).unwrap_or_else(|_| answer.to_string()))
        }
        _ => Err(not_in_family()),
    }
}

/// The trimmed, non-empty string argument `key`.
fn arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty())
}

/// What srv's `/api/v1/work-claims` takes: the claimed target, note and
/// lifetime, plus who is asking (srv prefers the request's agent token).
fn claim_body(arguments: &Value, agent: &str, block_id: &str) -> Value {
    let mut body = serde_json::Map::new();
    for key in ["path", "repo", "branch", "topic", "note"] {
        if let Some(v) = arg(arguments, key) {
            body.insert(key.into(), json!(v));
        }
    }
    if let Some(m) = arguments.get("ttl_minutes").and_then(|v| v.as_u64()) {
        body.insert("ttl_minutes".into(), json!(m));
    }
    body.insert("agent".into(), json!(agent.trim()));
    body.insert("block".into(), json!(block_id.trim()));
    Value::Object(body)
}

/// What srv's `/api/v1/work-claims/release` takes: a claim id, or none for all.
fn release_body(arguments: &Value, agent: &str, block_id: &str) -> Value {
    json!({ "id": arg(arguments, "id"), "agent": agent.trim(), "block": block_id.trim() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_given_arguments_and_the_callers_identity_are_sent() {
        let q = who_query(&json!({ "path": " crates/srv ", "query": "", "repo": null, "pr": 12 }), "AgentY", "blk");
        assert_eq!(
            q,
            vec![("path", "crates/srv".to_string()), ("agent", "AgentY".to_string()), ("block", "blk".to_string())]
        );
        assert!(who_query(&json!({}), "", "").is_empty());
    }

    #[test]
    fn claims_send_only_what_was_given_plus_the_callers_identity() {
        let body = claim_body(
            &json!({ "path": " crates/srv ", "topic": "", "note": "presence work", "ttl_minutes": 45, "repo": null }),
            "AgentY",
            "blk",
        );
        assert_eq!(
            body,
            json!({ "path": "crates/srv", "note": "presence work", "ttl_minutes": 45, "agent": "AgentY", "block": "blk" })
        );
        assert_eq!(release_body(&json!({}), "AgentY", "blk"), json!({ "id": null, "agent": "AgentY", "block": "blk" }));
        assert_eq!(release_body(&json!({ "id": "c1" }), "AgentY", "")["id"], "c1");
    }

    #[tokio::test]
    async fn without_agentmux_env_it_says_so() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        for tool in ["WhoIsWorkingOn", "ClaimWork", "ReleaseWork"] {
            let err = call(tool, &json!({ "path": "a" }), &cx).await.unwrap_err();
            assert!(err.to_string().contains("AGENTMUX_LOCAL_URL"), "{tool}");
        }
    }
}
