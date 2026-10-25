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
            let catch_up = arguments.get("catch_up").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
            let tz = arguments.get("tz").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
            let policy = arguments.get("policy").filter(|v| v.is_object());
            let template = arguments.get("template").filter(|v| !v.is_null());
            let params = arguments.get("params").filter(|v| v.is_object());
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty()).unwrap_or_default();
            // Identity M3 (spec §5.4): resolve the target while the author is
            // present; the job then fires by uid, never by resolving a name.
            let target_uid = resolve_agent_name_at_boundary(&client, local_url, auth_key, target).await?;

            let url = format!("{}/agentmux/jobs", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "name": name, "expression": expression, "prompt": prompt,
                "target": target, "target_uid": target_uid.unwrap_or_default(),
                "created_by": self_id, "max_fires": max_fires,
                "max_age_secs": max_age_secs, "catch_up": catch_up,
                "tz": tz, "policy": policy, "template": template, "params": params,
            });
            let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("cron create request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("Schedule create failed: HTTP {status} — {text}");
            }
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let job = &v["job"];
            // What srv changed in the request: a cap clamped to the default, a timeout added.
            let notes: String = v["notes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_str())
                .map(|n| format!("\nNote: {n}"))
                .collect();
            Ok(format!(
                "Created cron job '{}' (id={})\nExpression: {} {}\nNext fire: {}\nTarget: {}{notes}",
                job["name"].as_str().unwrap_or(name),
                job["id"].as_str().unwrap_or("?"),
                expression,
                job["tz"].as_str().unwrap_or("UTC"),
                job["next_fire"].as_str().unwrap_or("unknown"),
                target,
            ))
        }
        "CronDelete" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let url = format!("{}/agentmux/jobs/{}", local_url.trim_end_matches('/'), id);
            let resp = client.delete(&url).header(AUTH_KEY_HEADER, auth_key).send().await
                .map_err(|e| anyhow::anyhow!("cron delete request failed: {e}"))?;
            let status = resp.status();
            if status.as_u16() == 404 {
                return Ok(format!("Job '{id}' not found (already deleted or wrong id)"));
            }
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("Schedule delete failed: HTTP {status} — {text}");
            }
            Ok(format!("Deleted cron job {id}"))
        }
        "CronList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/agentmux/jobs", local_url.trim_end_matches('/'));
            let resp = client.get(&url).header(AUTH_KEY_HEADER, auth_key).send().await
                .map_err(|e| anyhow::anyhow!("cron list request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("Schedule list failed: HTTP {status} — {text}");
            }
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let jobs = v["jobs"].as_array().cloned().unwrap_or_default();
            // Pause all: the operator's switch over every job.
            let paused_all = v.get("paused_all").filter(|p| p.is_object()).map(|p| {
                format!(
                    "The operator paused every job (Pause all, {}): scheduled fires are skipped and run_now is refused until they resume them.",
                    ago(p["set_at_ms"].as_i64().unwrap_or(0))
                )
            });
            if jobs.is_empty() {
                return Ok(paused_all.map(|p| format!("{p}
No cron jobs configured.")).unwrap_or_else(|| "No cron jobs configured.".to_string()));
            }
            let mut lines: Vec<String> = paused_all.into_iter().collect();
            lines.push(format!("{} cron job(s):", jobs.len()));
            for j in &jobs {
                let status_str = if j["enabled"].as_bool().unwrap_or(false) { "enabled" } else { "paused" };
                let next = j["next_fire"].as_str().unwrap_or("—");
                let fires = j["fire_count"].as_i64().unwrap_or(0);
                let max = j["max_fires"].as_i64().map(|n| format!("/{n}")).unwrap_or_default();
                let age_bound = j["expires_in_secs"].as_i64().map(|n| format!("  expires_in={n}s")).unwrap_or_default();
                let last = j.get("last_run").filter(|r| r.is_object()).map(run_brief).unwrap_or_else(|| "none".to_string());
                // Scheduled fires that reached no one (not counted in fires=), and why the last did
                // A cap skips its runs today; they resume at the operator's midnight.
                let over_cap = j["over_cap"].as_str().map(|c| format!("  OVER CAP TODAY ({c})")).unwrap_or_default();
                let missed = match j["missed_count"].as_i64().unwrap_or(0) {
                    0 => String::new(),
                    n => format!("  missed={n} ({})", j["last_miss_reason"].as_str().unwrap_or("")),
                };
                lines.push(format!(
                    "  {}  {}  [{}]  fires={fires}{max}{missed}{over_cap}  next={}  expr='{}'{age_bound}\n      last run: {last}",
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
        "CronHistory" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let limit = arguments.get("limit").and_then(|v| v.as_i64()).filter(|&n| n > 0).unwrap_or(20);
            let url = format!("{}/agentmux/jobs/{}/runs?limit={limit}", local_url.trim_end_matches('/'), id);
            let resp = client.get(&url).header(AUTH_KEY_HEADER, auth_key).send().await
                .map_err(|e| anyhow::anyhow!("cron history request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("Schedule history failed: HTTP {status} — {text}");
            }
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let runs = v["runs"].as_array().cloned().unwrap_or_default();
            if runs.is_empty() {
                return Ok(format!("No runs recorded for job {id}."));
            }
            let mut lines = vec![format!("{} run(s) of job {id}, newest first:", runs.len())];
            lines.extend(runs.iter().map(|r| format!("  {}", run_brief(r))));
            Ok(lines.join("\n"))
        }
        "CronRunNow" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let url = format!("{}/agentmux/jobs/{}/run", local_url.trim_end_matches('/'), id);
            let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&serde_json::json!({})).send().await
                .map_err(|e| anyhow::anyhow!("cron run_now request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status.as_u16() == 404 {
                return Ok(format!("Job '{id}' not found"));
            }
            if !status.is_success() {
                anyhow::bail!("Schedule run_now failed: HTTP {status} — {text}");
            }
            let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let run_id = v["run_id"].as_str().unwrap_or("(not recorded)");
            match v["outcome"].as_str().unwrap_or("failed") {
                "delivered" => Ok(format!("Ran job {id} now: delivered (run {run_id})")),
                other => Ok(format!(
                    "Ran job {id} now: {other} (run {run_id}){}",
                    v["error"].as_str().map(|e| format!(" — {e}")).unwrap_or_default()
                )),
            }
        }
        "CronReport" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let run_id = arguments.get("run_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: run_id (from the line that ends the scheduled prompt)"))?;
            let outcome = arguments.get("outcome").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: outcome (ok, findings or failed)"))?;
            let summary = arguments.get("summary").and_then(|v| v.as_str()).unwrap_or("");
            let links = arguments.get("links").cloned().unwrap_or(serde_json::Value::Null);
            let url = format!("{}/agentmux/jobs/runs/{}/report", local_url.trim_end_matches('/'), run_id);
            let body = serde_json::json!({ "outcome": outcome, "summary": summary, "links": links });
            let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("cron report request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("Schedule report failed: HTTP {status} — {text}");
            }
            Ok(format!("Reported run {run_id}: {outcome}"))
        }
        _ => Err(not_in_family()),
    }
}

/// One run on one line: state (and reason), outcome, when, cost, summary.
fn run_brief(r: &serde_json::Value) -> String {
    let s = |k: &str| r[k].as_str().unwrap_or("");
    let mut state = s("state").to_string();
    if !s("reason").is_empty() {
        state = format!("{state} ({})", s("reason"));
    }
    let skips = r["skip_count"].as_i64().filter(|&n| n > 1).map(|n| format!(" x{n}")).unwrap_or_default();
    let when = r["scheduled_for_ms"].as_i64().or_else(|| r["started_at_ms"].as_i64()).map(ago).unwrap_or_default();
    let cost = match (r["cost_known"].as_bool(), r["cost_usd"].as_f64()) {
        (Some(true), Some(c)) => format!("  ${c:.2}{}", if r["cost_partial"].as_bool() == Some(true) { " (partial)" } else { "" }),
        _ => String::new(),
    };
    let summary = if s("summary").is_empty() { String::new() } else { format!("  \"{}\"", s("summary")) };
    // Only when it isn't ok: an interrupted run must not read like an
    // ordinary unreported one.
    let infra = match s("infra") {
        "" | "ok" => String::new(),
        other => format!("  infra={other}"),
    };
    format!("{}  {state}{skips}{infra}  outcome={}  {when}{cost}{summary}", s("id"), s("outcome"))
}

/// A ms epoch relative to now: "12 min ago", "in 3 h".
fn ago(ms: i64) -> String {
    let diff = agentmux_common::time::now_ms() - ms;
    let mins = diff.abs() / 60_000;
    let span = match mins {
        0 => return "just now".to_string(),
        m if m < 120 => format!("{m} min"),
        m if m < 48 * 60 => format!("{} h", m / 60),
        m => format!("{} d", m / (24 * 60)),
    };
    if diff >= 0 { format!("{span} ago") } else { format!("in {span}") }
}

#[cfg(test)]
mod tests {
    use super::run_brief;

    #[test]
    fn a_run_line_names_its_infra_status_unless_ok() {
        let run = |infra: &str| {
            serde_json::json!({ "id": "r1", "state": "finished", "infra": infra, "outcome": "unknown", "skip_count": 0 })
        };
        assert!(run_brief(&run("interrupted")).contains("finished  infra=interrupted  outcome=unknown"), "{}", run_brief(&run("interrupted")));
        assert!(run_brief(&run("error")).contains("infra=error"));
        for quiet in ["ok", ""] {
            let line = run_brief(&run(quiet));
            assert!(line.contains("finished  outcome=unknown") && !line.contains("infra="), "{line}");
        }
    }
}
