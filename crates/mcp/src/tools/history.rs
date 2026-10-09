// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Conversation history search and transcripts.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, client, .. } = *cx;
    match name {
        "SearchHistory" => {
            // Whose history this is, srv takes from this process's
            // `X-Agent-Token` (sent on every request, see `main`), never from a
            // name; the tool exposes no `agent` parameter on purpose — see
            // SEARCH_HISTORY_TOOL's comment. The name is sent only for srv's
            // actor counters, so a process without one still searches.
            let slug = std::env::var("AGENTMUX_AGENT_ID").unwrap_or_default();
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let tool = arguments.get("tool").and_then(|v| v.as_str());
            if query.trim().is_empty() && tool.is_none() {
                anyhow::bail!(
                    "query must be non-empty unless `tool` is given (an empty query with no \
                     tool filter would match everything and return a truncated firehose)"
                );
            }

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!(
                "{}/agentmux/reactive/history/search",
                local_url.trim_end_matches('/')
            );
            let mut query_params: Vec<(&str, String)> = vec![("query", query)];
            if !slug.trim().is_empty() {
                query_params.push(("agent", slug));
            }
            if let Some(t) = tool {
                query_params.push(("tool", t.to_string()));
            }
            if let Some(r) = arguments.get("role").and_then(|v| v.as_str()) {
                query_params.push(("role", r.to_string()));
            }
            for (key, arg) in [
                ("since", "since"),
                ("until", "until"),
                ("max_sessions", "max_sessions"),
                ("limit", "limit"),
            ] {
                if let Some(n) = arguments.get(arg).and_then(|v| v.as_i64()) {
                    query_params.push((key, n.to_string()));
                }
            }
            if arguments.get("include_inferred").and_then(|v| v.as_bool()) == Some(true) {
                query_params.push(("include_inferred", "true".to_string()));
            }

            let body = srv_get_json(client, &url, auth_key, &query_params, "history search").await?;
            Ok(serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string()))
        }
        "GetAgentTranscript" => {
            let agent = arguments
                .get("agent")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: agent"))?;
            let max_lines = arguments.get("max_lines").and_then(|v| v.as_u64());

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!(
                "{}/agentmux/reactive/transcript",
                local_url.trim_end_matches('/')
            );
            let mut query: Vec<(&str, String)> = vec![("agent", agent.to_string())];
            if let Some(n) = max_lines {
                query.push(("max_lines", n.to_string()));
            }

            let resp = client
                .get(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .query(&query)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("transcript fetch failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "ListConversations" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!(
                "{}/api/v1/muxspect/conversations",
                local_url.trim_end_matches('/')
            );
            let resp = client
                .get(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("conversations fetch failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        _ => Err(not_in_family()),
    }
}
