// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-process prompt loops (Loop, LoopStop, LoopList).

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, client, loops, loop_counter, .. } = *cx;
    match name {
        "Loop" => {
            let prompt = arguments
                .get("prompt")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: prompt"))?
                .to_string();

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let self_id = std::env::var("AGENTMUX_AGENT_ID")
                .ok()
                .filter(|s| !s.is_empty());
            let target = arguments
                .get("to")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .or_else(|| self_id.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no loop target: pass `to`, or ensure AGENTMUX_AGENT_ID is set for a self-loop"
                    )
                })?;

            let interval_str = arguments
                .get("interval")
                .and_then(|v| v.as_str())
                .unwrap_or("10m")
                .to_string();
            let interval = parse_interval(&interval_str)?;
            let immediate = arguments
                .get("immediate")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let max_iterations: Option<u64> = arguments
                .get("max_iterations")
                .and_then(|v| v.as_u64())
                .filter(|&n| n > 0);

            let n = loop_counter.fetch_add(1, Ordering::Relaxed) + 1;
            let loop_id = format!("loop-{n}");

            let interval_display = format_duration(interval);
            let url = format!("{}/agentmux/reactive/inject", local_url.trim_end_matches('/'));
            let task_client = client.clone();
            let task_auth = auth_key.to_string();
            let task_target = target.clone();
            let task_source = self_id;
            let task_prompt = prompt.clone();
            let fire_count = Arc::new(AtomicU64::new(0));
            let task_fire_count = Arc::clone(&fire_count);
            let task_max = max_iterations;

            let handle = tokio::spawn(async move {
                if !immediate {
                    tokio::time::sleep(interval).await;
                }
                loop {
                    let req = sign_outgoing_jekt(task_source.as_deref(), &task_target, &task_prompt)
                        .into_request(task_target.clone(), task_prompt.clone(), task_source.clone());
                    let _ = task_client
                        .post(&url)
                        .header("X-AuthKey", &task_auth)
                        .json(&req)
                        .send()
                        .await;
                    let fired = task_fire_count.fetch_add(1, Ordering::Relaxed) + 1;
                    if let Some(max) = task_max {
                        if fired >= max {
                            break;
                        }
                    }
                    tokio::time::sleep(interval).await;
                }
            });

            let started_at = agentmux_common::time::now_secs_u64();

            loops.lock().unwrap().insert(loop_id.clone(), LoopEntry {
                handle,
                prompt,
                target: target.clone(),
                interval_secs: interval.as_secs(),
                max_iterations,
                fire_count,
                started_at,
            });

            let cap_note = match max_iterations {
                Some(n) => format!(", auto-stops after {n} fires"),
                None => String::new(),
            };
            Ok(format!(
                "Started {loop_id}: injecting to '{target}' every {interval_display}{cap_note}\
                 {}. Stop with LoopStop({loop_id}) or use LoopList() to see all running loops.",
                if immediate { ", first run now" } else { "" }
            ))
        }
        "LoopStop" => {
            let loop_id = arguments
                .get("loop_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: loop_id"))?;

            let removed = loops.lock().unwrap().remove(loop_id);
            match removed {
                Some(entry) => {
                    entry.handle.abort();
                    Ok(format!(
                        "stopped {loop_id} (fired {} time(s))",
                        entry.fire_count.load(Ordering::Relaxed)
                    ))
                }
                None => Ok(format!("{loop_id} was not running (unknown or already stopped)")),
            }
        }
        "LoopList" => {
            let reg = loops.lock().unwrap();
            if reg.is_empty() {
                return Ok("No loops running in this session.".to_string());
            }
            let mut lines = vec![format!("{} loop(s) in this session:", reg.len())];
            for (id, entry) in reg.iter() {
                let fired = entry.fire_count.load(Ordering::Relaxed);
                let status = match entry.max_iterations {
                    Some(max) if fired >= max => format!("DONE ({fired}/{max})"),
                    Some(max) => format!("running ({fired}/{max})"),
                    None => format!("running ({fired} fired, unlimited)"),
                };
                let interval = format_duration(Duration::from_secs(entry.interval_secs));
                let prompt_preview: String = entry.prompt.chars().take(60).collect();
                let age_secs = agentmux_common::time::now_secs_u64().saturating_sub(entry.started_at);
                lines.push(format!(
                    "  {id}  every={interval}  to='{}'  status={status}  age={age_secs}s  prompt='{prompt_preview}'",
                    entry.target,
                ));
            }
            Ok(lines.join("\n"))
        }
        _ => Err(not_in_family()),
    }
}
