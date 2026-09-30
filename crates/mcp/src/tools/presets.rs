// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Presets.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "PresetList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/preset/list", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("preset/list failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "PresetGet" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/preset/get", local_url.trim_end_matches('/'));
            // id/name are both optional; with neither set the server returns the
            // agent's own bound preset ("self"), resolved from agent_id.
            let mut query: Vec<(&str, &str)> = vec![("agent_id", agent_id.as_str())];
            if let Some(pid) = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                query.push(("id", pid));
            }
            if let Some(pname) = arguments.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                query.push(("name", pname));
            }
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&query)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("preset/get failed: HTTP {status} — {text}");
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
