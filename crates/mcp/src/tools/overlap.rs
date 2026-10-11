// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who else is working on the same thing
//! (docs/specs/SPEC_AGENT_OVERLAP_AWARENESS_2026_10_10.md §3.2).

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
        _ => Err(not_in_family()),
    }
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

    #[tokio::test]
    async fn without_agentmux_env_it_says_so() {
        let client = reqwest::Client::new();
        let loops = LoopRegistry::default();
        let counter = std::sync::atomic::AtomicU64::new(0);
        let cx = ToolCtx { local_url: "", auth_key: "", block_id: "", client: &client, loops: &loops, loop_counter: &counter };
        let err = call("WhoIsWorkingOn", &json!({ "path": "a" }), &cx).await.unwrap_err();
        assert!(err.to_string().contains("AGENTMUX_LOCAL_URL"));
    }
}
