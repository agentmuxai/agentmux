// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-to-agent messaging and fleet operations.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, client, .. } = *cx;
    match name {
        "SendMessage" => {
            let to = arguments
                .get("to")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: to"))?;
            let message = arguments
                .get("message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: message"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let source_agent = std::env::var("AGENTMUX_AGENT_ID")
                .ok()
                .filter(|s| !s.is_empty());

            let url = format!(
                "{}/agentmux/reactive/inject",
                local_url.trim_end_matches('/')
            );
            let req = sign_outgoing_jekt(source_agent.as_deref(), to, message)
                .into_request(to.to_string(), message.to_string(), source_agent);

            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&req)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("inject failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            send_message_outcome(to, &result)
        }
        "DiscoverAgents" => {
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/agentmux/discovery", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("discovery failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "FleetList" => {
            // Thin fleet-framed alias of DiscoverAgents — identical call,
            // see this tool's own doc comment for why a separate tool
            // exists despite reusing the exact same endpoint.
            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/agentmux/discovery", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("discovery failed: HTTP {status} — {text}");
            }

            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "FleetBroadcast" => {
            let targets = arguments
                .get("targets")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: targets"))?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            if targets.is_empty() {
                anyhow::bail!("targets must be a non-empty array of block_id (host/cross-channel) or agent name (LAN/WAN) strings");
            }
            let message = arguments
                .get("message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: message"))?;

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }
            let source_agent = std::env::var("AGENTMUX_AGENT_ID")
                .ok()
                .filter(|s| !s.is_empty());

            // `/agentmux/reactive/inject`'s `target_agent` resolves by
            // agent UID or registered AGENT NAME only (`resolve_target`,
            // crates/srv/src/backend/reactive/handler.rs) — never by
            // block_id, even though this tool's own advertised contract
            // (FLEET_BROADCAST_TOOL) is block_id values from FleetList, to
            // stay consistent with FleetBulkStop's targeting scheme. The
            // WS-RPC path (`fleet_broadcast_impl`, agentmux-srv) already
            // resolves block_id -> agent_id via `get_agent_by_block` before
            // injecting; this MCP path talks to srv over plain HTTP with no
            // access to that in-process registry, so it resolves the same
            // way DiscoverAgents/FleetList already do: read
            // `/agentmux/discovery`'s `host.addressable` (each entry already
            // carries both `agent_id` and a live `block_id`) and map through
            // it before signing/injecting (Codex P1 + reagent P0, PR #2687
            // review — every advertised call failed "agent not found"
            // without this).
            //
            // REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md:
            // `host.addressable` alone missed `host.cross_channel` entries
            // (a different channel on this SAME host — those genuinely have
            // a `block_id` in this host's namespace, discovery just wasn't
            // being read for it) and LAN/WAN entries (which have NO
            // `block_id` at all — only an agent name, since they're not
            // local blocks). Both are folded in below: `cross_channel`
            // extends the same block_id->name map (identical shape), and
            // any target string that doesn't match ANY known block_id falls
            // through to being used AS a literal agent name — safe because
            // `/agentmux/reactive/inject`'s own cross-tier cascade
            // (cross-channel -> LAN -> WAN muxbus relay,
            // `server/reactive.rs`) already resolves by name across every
            // tier; an invalid name just fails the same "not found" way it
            // always did.
            let discovery_url = format!("{}/agentmux/discovery", local_url.trim_end_matches('/'));
            let discovery_resp = client
                .get(&discovery_url)
                .header(AUTH_KEY_HEADER, auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("discovery request failed: {e}"))?;
            if !discovery_resp.status().is_success() {
                let status = discovery_resp.status();
                let text = discovery_resp.text().await.unwrap_or_default();
                anyhow::bail!("discovery failed: HTTP {status} — {text}");
            }
            let discovery: Value = discovery_resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("discovery response parse failed: {e}"))?;
            let block_to_agent = build_block_to_agent_map(&discovery);

            // Same signed single-target delivery SendMessage uses, looped
            // once per target — only this process holds AGENTMUX_JEKT_KEY,
            // so per-message signing can only happen here (see this
            // tool's own doc comment). Never a single aggregate result:
            // per-target success/failure is collected below regardless of
            // how many targets fail. Sent in chunks with a pause between
            // them — `/agentmux/reactive/inject` shares ReactiveHandler's
            // global rate limiter (10/sec, hard reset per second, not a
            // smooth refill — crates/srv/src/backend/reactive/mod.rs's
            // RATE_LIMIT_MAX), and a tight loop past ~10 targets would
            // otherwise deterministically fail the tail of any larger
            // broadcast with "rate limit exceeded" (Codex P1, same review).
            // Mirrors `fleet_broadcast_impl`'s own chunking constants —
            // kept in sync by hand since this is a separate process/crate
            // with no dependency on agentmux-srv internals.
            const BROADCAST_CHUNK_SIZE: usize = 10;
            const BROADCAST_CHUNK_PAUSE: std::time::Duration = std::time::Duration::from_millis(1100);

            let url = format!("{}/agentmux/reactive/inject", local_url.trim_end_matches('/'));
            let mut succeeded: Vec<String> = Vec::new();
            let mut failed: Vec<serde_json::Value> = Vec::new();
            for (chunk_idx, chunk) in targets.chunks(BROADCAST_CHUNK_SIZE).enumerate() {
                if chunk_idx > 0 {
                    tokio::time::sleep(BROADCAST_CHUNK_PAUSE).await;
                }
                for target in chunk {
                    let target = target.clone();
                    // LAN/WAN discovery entries carry no block_id at all
                    // (they're not local blocks) — a target that doesn't
                    // match any known block_id is used AS the agent name
                    // directly, letting `/agentmux/reactive/inject`'s own
                    // cross-tier cascade attempt it. This can never make a
                    // genuinely-wrong block_id succeed silently: it still
                    // fails, just via the inject endpoint's own "agent not
                    // found" rather than this pre-check.
                    let target_agent = block_to_agent.get(&target).cloned().unwrap_or_else(|| target.clone());
                    let req = sign_outgoing_jekt(source_agent.as_deref(), &target_agent, message)
                        .into_request(target_agent, message.to_string(), source_agent.clone());
                    let outcome = async {
                        let resp = client
                            .post(&url)
                            .header(AUTH_KEY_HEADER, auth_key)
                            .json(&req)
                            .send()
                            .await
                            .map_err(|e| format!("request failed: {e}"))?;
                        if !resp.status().is_success() {
                            let status = resp.status();
                            let text = resp.text().await.unwrap_or_default();
                            return Err(format!("HTTP {status} — {text}"));
                        }
                        let result: Value = resp
                            .json()
                            .await
                            .map_err(|e| format!("response parse failed: {e}"))?;
                        if result.get("success").and_then(|v| v.as_bool()) == Some(true) {
                            Ok(())
                        } else {
                            Err(result
                                .get("error")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown error")
                                .to_string())
                        }
                    }
                    .await;
                    match outcome {
                        Ok(()) => succeeded.push(target),
                        Err(error) => failed.push(json!({ "id": target, "error": error })),
                    }
                }
            }

            let result = json!({ "succeeded": succeeded, "failed": failed });
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "FleetBulkStop" => {
            let targets = arguments
                .get("targets")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: targets"))?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            if targets.is_empty() {
                anyhow::bail!("targets must be a non-empty array of block_id strings");
            }
            let signal = arguments.get("signal").and_then(|v| v.as_str()).map(str::to_string);
            let staged = arguments.get("staged").cloned();

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let url = format!("{}/api/v1/fleet/bulk-stop", local_url.trim_end_matches('/'));
            let mut body = json!({ "targets": targets, "signal": signal });
            if let Some(staged) = staged {
                body["staged"] = staged;
            }
            // Names this agent on the user's banner and in the audit log.
            if let Ok(auth) = sign_ui_automation_auth() {
                body["auth"] = serde_json::to_value(auth).unwrap_or(Value::Null);
            }
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("fleet bulk-stop failed: HTTP {status} — {text}");
            }

            let mut result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            if result["status"] == "pending_user_override" {
                // Each running target's user had 15 s to keep it (§6.5). srv
                // runs the windows in parallel, so waiting on them one after
                // another costs no extra time.
                let pending = result["pending"].as_array().cloned().unwrap_or_default();
                let mut outcomes = Vec::with_capacity(pending.len());
                for p in &pending {
                    let request_id = p["request_id"].as_str().unwrap_or_default();
                    let now = await_shutdown(client, local_url, auth_key, request_id, true).await;
                    outcomes.push((p["block_id"].as_str().unwrap_or_default().to_string(), now));
                }
                result = merge_bulk_stop_outcomes(result, outcomes);
            }

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "SupervisorNudge" => {
            let target_agent = arguments
                .get("target_agent")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: target_agent"))?;
            let action = arguments
                .get("action")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;
            if action != "nudge" && action != "decline" {
                anyhow::bail!("invalid action: {action} (expected \"nudge\" or \"decline\")");
            }
            let reason = arguments.get("reason").and_then(|v| v.as_str());

            if local_url.is_empty() || auth_key.is_empty() {
                anyhow::bail!(
                    "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
                     Is this agent pane opened via AgentMux?"
                );
            }

            let source_agent = std::env::var("AGENTMUX_AGENT_ID")
                .ok()
                .filter(|s| !s.is_empty());

            let url = format!(
                "{}/agentmux/reactive/supervisor-decision",
                local_url.trim_end_matches('/')
            );
            let body = json!({
                "target_agent": target_agent,
                "action": action,
                "reason": reason,
                "source_agent": source_agent,
            });

            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;

            let status = resp.status();
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;

            if !status.is_success() {
                let err = result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                anyhow::bail!("supervisor decision rejected: HTTP {status} — {err}");
            }

            // The route always returns HTTP 200 for a request that passed
            // the entitlement/ceiling gates — actual delivery success is
            // only reflected in the response body's `success` field (a
            // failed nudge still gets logged and returned as 200/success:
            // false, same as SendMessage's InjectionResponse). Checking
            // HTTP status alone previously reported a failed delivery to
            // the calling Supervisor as success (reagentx P2 on PR #2557).
            if result.get("success").and_then(|v| v.as_bool()) != Some(true) {
                let err = result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                anyhow::bail!("supervisor decision delivery failed: {err}");
            }

            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        _ => Err(not_in_family()),
    }
}
