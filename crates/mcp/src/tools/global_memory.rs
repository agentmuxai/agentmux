// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Global Memory.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "GlobalMemoryList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/list", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/list failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "GlobalMemoryRead" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/read", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("id", id)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/read failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(result
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                }))
        }
        "GlobalMemoryWrite" => {
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: name"))?;
            let content = arguments
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/globalmemory/write", local_url.trim_end_matches('/'));
            let mut body = json!({
                "agent_id": agent_id,
                "name": name,
                "content": content,
            });
            if let Some(id) = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                body["id"] = json!(id);
            }
            // Pass provenance through verbatim when the caller supplied it —
            // same advisory-metadata pattern as MemoryWrite above.
            if let Some(provenance) = arguments.get("provenance") {
                body["provenance"] = provenance.clone();
            }
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/write failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let id = result.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            Ok(format!("Wrote Global Memory entry \"{name}\" (id: {id})"))
        }
        "GlobalMemoryRemove" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/remove", local_url.trim_end_matches('/'));
            let body = json!({ "id": id });
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/remove failed: HTTP {status} — {text}");
            }
            Ok(format!("Removed Global Memory entry {id}"))
        }
        "GlobalMemoryHistory" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/history", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("id", id)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/history failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "GlobalMemoryDiff" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let from_version_id = arguments
                .get("from_version_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: from_version_id"))?;
            let to_version_id = arguments
                .get("to_version_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: to_version_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/api/v1/agent/globalmemory/diff", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header("X-AuthKey", auth_key)
                .query(&[("id", id), ("from_version_id", from_version_id), ("to_version_id", to_version_id)])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/diff failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(result
                .get("diff")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                }))
        }
        "GlobalMemoryRevert" => {
            let id = arguments
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let version_id = arguments
                .get("version_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: version_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/globalmemory/revert", local_url.trim_end_matches('/'));
            let body = json!({
                "agent_id": agent_id,
                "id": id,
                "version_id": version_id,
            });
            let resp = client
                .post(&url)
                .header("X-AuthKey", auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("globalmemory/revert failed: HTTP {status} — {text}");
            }
            Ok(format!("Reverted Global Memory entry {id} to version {version_id}"))
        }
        _ => Err(not_in_family()),
    }
}
