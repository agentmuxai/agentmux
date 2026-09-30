// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Scheduled prompts (Cron*).

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "CronCreate" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let name = arguments.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: name"))?;
            let expression = arguments.get("expression").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: expression"))?;
            let prompt = arguments.get("prompt").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: prompt"))?;
            let target = arguments.get("to").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: to (target agent id)"))?;
            let max_fires = arguments.get("max_fires").and_then(|v| v.as_i64()).filter(|&n| n > 0);
            let max_age_secs = arguments.get("max_age_secs").and_then(|v| v.as_i64()).filter(|&n| n > 0);
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty()).unwrap_or_default();
            // Identity M3 (spec §5.4): resolve the target while the author is
            // present; the job then fires by uid, never by resolving a name.
            let target_uid = resolve_agent_name_at_boundary(&client, local_url, auth_key, target).await?;

            let url = format!("{}/agentmux/cron", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "name": name, "expression": expression, "prompt": prompt,
                "target": target, "target_uid": target_uid.unwrap_or_default(),
                "created_by": self_id, "max_fires": max_fires,
                "max_age_secs": max_age_secs,
            });
            let resp = client.post(&url).header("X-AuthKey", auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("cron create request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("CronCreate failed: HTTP {status} — {text}");
            }
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let job = &v["job"];
            Ok(format!(
                "Created cron job '{}' (id={})\nExpression: {} UTC\nNext fire: {}\nTarget: {}",
                job["name"].as_str().unwrap_or(name),
                job["id"].as_str().unwrap_or("?"),
                expression,
                job["next_fire"].as_str().unwrap_or("unknown"),
                target,
            ))
        }
        "CronDelete" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let url = format!("{}/agentmux/cron/{}", local_url.trim_end_matches('/'), id);
            let resp = client.delete(&url).header("X-AuthKey", auth_key).send().await
                .map_err(|e| anyhow::anyhow!("cron delete request failed: {e}"))?;
            let status = resp.status();
            if status.as_u16() == 404 {
                return Ok(format!("Job '{id}' not found (already deleted or wrong id)"));
            }
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("CronDelete failed: HTTP {status} — {text}");
            }
            Ok(format!("Deleted cron job {id}"))
        }
        "CronList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/agentmux/cron", local_url.trim_end_matches('/'));
            let resp = client.get(&url).header("X-AuthKey", auth_key).send().await
                .map_err(|e| anyhow::anyhow!("cron list request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("CronList failed: HTTP {status} — {text}");
            }
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let jobs = v["jobs"].as_array().cloned().unwrap_or_default();
            if jobs.is_empty() {
                return Ok("No cron jobs configured.".to_string());
            }
            let mut lines = vec![format!("{} cron job(s):", jobs.len())];
            for j in &jobs {
                let status_str = if j["enabled"].as_bool().unwrap_or(false) { "enabled" } else { "paused" };
                let next = j["next_fire"].as_str().unwrap_or("—");
                let fires = j["fire_count"].as_i64().unwrap_or(0);
                let max = j["max_fires"].as_i64().map(|n| format!("/{n}")).unwrap_or_default();
                let age_bound = j["expires_in_secs"].as_i64().map(|n| format!("  expires_in={n}s")).unwrap_or_default();
                lines.push(format!(
                    "  {}  {}  [{}]  fires={fires}{max}  next={}  expr='{}'{age_bound}",
                    j["id"].as_str().unwrap_or("?"),
                    j["name"].as_str().unwrap_or("?"),
                    status_str,
                    next,
                    j["expression"].as_str().unwrap_or("?"),
                ));
            }
            Ok(lines.join("\n"))
        }
        "CronPause" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            cron_set_enabled(client, local_url, auth_key, id, "pause").await
        }
        "CronResume" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            cron_set_enabled(client, local_url, auth_key, id, "resume").await
        }
        _ => Err(not_in_family()),
    }
}
