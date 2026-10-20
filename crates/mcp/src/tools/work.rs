// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shared work queue (Work*).

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        // ── Muxqueue ────────────────────────────────────────────────────
        // All six post/get to /agentmux/work* on the local srv, same auth
        // header as the cron arms below.
        "WorkEnqueue" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let title = arguments.get("title").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: title"))?;
            let payload = arguments.get("payload").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: payload"))?;
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty()).unwrap_or_default();
            // Kanban §6.5: `for: "me"` files it in the caller's own backlog,
            // to take after its current work.
            let for_me = arguments.get("for").and_then(|v| v.as_str()).map(str::trim) == Some("me");
            if for_me && self_id.is_empty() {
                anyhow::bail!("for \"me\" needs AGENTMUX_AGENT_ID, which is not set");
            }
            let target_agent = if for_me {
                self_id.as_str()
            } else {
                arguments.get("target_agent").and_then(|v| v.as_str()).unwrap_or("")
            };
            // Identity M3: resolve the typed target NOW, at the boundary, and
            // carry its uid; an ambiguous name is handed back with candidates.
            // The caller itself is its own UID.
            let target_agent_uid = if target_agent.is_empty() {
                None
            } else if for_me {
                std::env::var("AGENTMUX_AGENT_UID").ok().filter(|s| !s.is_empty())
            } else {
                resolve_agent_name_at_boundary(&client, local_url, auth_key, target_agent).await?
            };

            let url = format!("{}/agentmux/work", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "title": title,
                "payload": payload,
                "kind": arguments.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
                "target_agent": target_agent,
                "target_agent_uid": target_agent_uid.unwrap_or_default(),
                "target_group": arguments.get("target_group").and_then(|v| v.as_str()).unwrap_or(""),
                "priority": arguments.get("priority").and_then(|v| v.as_i64()).unwrap_or(0),
                "not_before": arguments.get("not_before").and_then(|v| v.as_i64()),
                "max_attempts": arguments.get("max_attempts").and_then(|v| v.as_i64()).filter(|&n| n > 0),
                "created_by": self_id,
                "repo": arguments.get("repo").and_then(|v| v.as_str()),
                "hold": arguments.get("hold").and_then(|v| v.as_bool()).unwrap_or(false),
                "review": arguments.get("review").and_then(|v| v.as_bool()),
            });
            let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("work enqueue request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("Tasks enqueue failed: HTTP {status} — {text}");
            }
            let v = serde_json::from_str::<Value>(&text).unwrap_or(serde_json::json!({}));
            let id = v.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string();
            let name = task_name(&v, &id);
            Ok(if for_me {
                format!(
                    "Filed {name} in your backlog: {title}. Tell the operator its number. It waits for you: \
                     finish what you are doing, then take it with Tasks claim."
                )
            } else {
                format!("Enqueued {name}: {title}")
            })
        }
        "WorkClaim" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("AGENTMUX_AGENT_ID is not set — cannot claim work without an agent identity"))?;

            // Identity M1b/M3: carry this agent's UID (AGENTMUX_AGENT_UID, set
            // by srv at spawn since M1a). It is the only way into a
            // UID-addressed item. Empty when absent (pre-M1a spawn,
            // continuation resume, quick-launch pane) — srv then takes the
            // UID from this block's row, and counts the fallback.
            let self_uid = std::env::var("AGENTMUX_AGENT_UID").ok().filter(|s| !s.is_empty()).unwrap_or_default();

            let url = format!("{}/agentmux/work/claim", local_url.trim_end_matches('/'));
            let body = serde_json::json!({
                "agent_id": self_id,
                "agent_uid": self_uid,
                // Identity M3: when no UID is carried, srv takes it from the
                // row on this block rather than deriving one from the name.
                "block_id": block_id,
                "kind": arguments.get("kind").and_then(|v| v.as_str()),
                "lease_ms": arguments.get("lease_ms").and_then(|v| v.as_i64()).filter(|&n| n > 0),
                // Group membership is resolved server-side-of-this-call by the
                // caller in the general design; the MCP path has no cheap way
                // to know this agent's groups yet, so it claims only untargeted
                // and self-targeted work. Group-targeted claiming arrives with
                // the group lookup, not before — better to under-claim than to
                // silently ignore a group restriction.
                "groups": [],
            });
            let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("work claim request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("Tasks claim failed: HTTP {status} — {text}");
            }
            let v: Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
            if !v.get("claimed").and_then(|x| x.as_bool()).unwrap_or(false) {
                return Ok("No work available on the queue right now.".to_string());
            }
            let attempt = v.get("attempt").and_then(|x| x.as_i64()).unwrap_or(0);
            let item = v.get("item").cloned().unwrap_or(serde_json::json!({}));
            let id = item.get("id").and_then(|x| x.as_str()).unwrap_or("");
            let name = task_name(&v, id);
            let title = item.get("title").and_then(|x| x.as_str()).unwrap_or("");
            let payload = item.get("payload").and_then(|x| x.as_str()).unwrap_or("");
            // Carry forward why a PREVIOUS holder handed this back (Codex P2 on
            // PR #2902). WorkRelease's description promises the next claimant
            // sees the reason; dropping it here broke that promise and let
            // agents re-hit a known blocker with no warning. `attempt > 1` is
            // exactly "someone has held this before me".
            let prior = item.get("result").and_then(|x| x.as_str()).unwrap_or("");
            let handback = if attempt > 1 && !prior.is_empty() {
                format!(
                    "\n\nNOTE — a previous agent held this and handed it back \
                     (attempt {} of this item). Their reason: {prior}\n\
                     Read that before repeating their approach.",
                    attempt - 1
                )
            } else {
                String::new()
            };
            let short = v.get("num").and_then(|n| n.as_i64()).map(|n| n.to_string()).unwrap_or_else(|| id.to_string());
            Ok(format!(
                "Claimed {name} (attempt {attempt}): pass id={short} and attempt={attempt} to \
                 Tasks update, ask, submit, complete and release for this task.\n\n\
                 Title: {title}\n\n{payload}{handback}"
            ))
        }
        "WorkUpdate" | "WorkAsk" | "WorkSubmit" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("AGENTMUX_AGENT_ID is not set"))?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let attempt = arguments.get("attempt").and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: attempt (from the Tasks claim answer)"))?;
            let str_arg = |k: &str| arguments.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
            let (segment, text) = match name {
                "WorkAsk" => ("ask", str_arg("question").ok_or_else(|| anyhow::anyhow!("Tasks ask needs a question"))?),
                "WorkSubmit" => (
                    "submit",
                    str_arg("result").ok_or_else(|| anyhow::anyhow!("Tasks submit needs a result: what was done, with links"))?,
                ),
                _ => ("update", ""),
            };
            let links = arguments.get("links").cloned().unwrap_or(Value::Null);
            let url = format!("{}/agentmux/work/{}/{}", local_url.trim_end_matches('/'), id, segment);
            let body = serde_json::json!({
                "agent_id": self_id,
                "attempt": attempt,
                "note": str_arg("note"),
                "checklist": arguments.get("checklist").filter(|c| c.is_array()),
                "pr_url": links.get("pr").and_then(|v| v.as_str()),
                "branch": links.get("branch").and_then(|v| v.as_str()),
                "text": text,
            });
            let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("work {segment} request failed: {e}"))?;
            let status = resp.status();
            let reply = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::CONFLICT {
                anyhow::bail!(
                    "This task is no longer yours, or that attempt number is old. Check it with \
                     Tasks list; claim again if you still want work."
                );
            }
            if !status.is_success() {
                anyhow::bail!("Tasks {segment} failed: HTTP {status} — {reply}");
            }
            let v: Value = serde_json::from_str(&reply).unwrap_or(serde_json::json!({}));
            Ok(match segment {
                "ask" => format!(
                    "Asked: task {id} now waits in the operator's Needs you column, and stays yours. \
                     Their answer comes to you as a message."
                ),
                "submit" => finished_text(id, &v),
                _ => format!("Updated task {id}."),
            })
        }
        "WorkHeartbeat" | "WorkComplete" | "WorkRelease" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let self_id = std::env::var("AGENTMUX_AGENT_ID").ok().filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("AGENTMUX_AGENT_ID is not set"))?;
            let id = arguments.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: id"))?;
            let attempt = arguments.get("attempt").and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: attempt (from the Tasks claim answer)"))?;

            let (segment, result_text) = match name {
                "WorkHeartbeat" => ("heartbeat", String::new()),
                "WorkComplete" => (
                    "complete",
                    // Enforced at runtime as well as in the schema (Codex P2 on
                    // PR #2902): completing with no result marks the item done
                    // forever while destroying the only record that the work
                    // happened. Better to reject the call than to accept a
                    // silent hole in the audit trail.
                    arguments
                        .get("result")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Tasks complete requires a non-empty 'result' — it is the only \
                                 record of what this item accomplished once it is marked done"
                            )
                        })?
                        .to_string(),
                ),
                _ => (
                    "release",
                    arguments.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                ),
            };

            let url = format!("{}/agentmux/work/{}/{}", local_url.trim_end_matches('/'), id, segment);
            let body = serde_json::json!({
                "agent_id": self_id,
                "attempt": attempt,
                "result": result_text,
                "lease_ms": arguments.get("lease_ms").and_then(|v| v.as_i64()).filter(|&n| n > 0),
            });
            let resp = client.post(&url).header(AUTH_KEY_HEADER, auth_key).json(&body).send().await
                .map_err(|e| anyhow::anyhow!("work {segment} request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::CONFLICT {
                // The fence rejected this call. Say so in the terms the agent
                // can act on, rather than surfacing a raw 409 — the recovery is
                // always the same: claim again.
                anyhow::bail!(
                    "This claim is no longer yours — the lease expired and the item was \
                     reclaimed, or another agent holds it now. Claim again with Tasks if you \
                     still want work; do NOT retry with the old attempt number."
                );
            }
            if !status.is_success() {
                anyhow::bail!("{name} failed: HTTP {status} — {text}");
            }
            Ok(match segment {
                "heartbeat" => format!("Lease extended on {id}."),
                "complete" => finished_text(id, &serde_json::from_str::<Value>(&text).unwrap_or(serde_json::json!({}))),
                _ => {
                    // A release on the FINAL allowed attempt parks the item as
                    // `failed` rather than reopening it, so reporting "back to
                    // the queue" unconditionally would tell the caller another
                    // agent can pick it up when nobody ever will (Codex P2 on
                    // PR #2902). The server reports the resulting state; trust
                    // it rather than re-deriving the attempts rule here.
                    let resulting = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|v| v.get("state").and_then(|s| s.as_str()).map(str::to_string))
                        .unwrap_or_default();
                    if resulting == "failed" {
                        format!(
                            "Released {id}, and it has now used its final attempt — the item is \
                             parked as FAILED and will not be offered to any agent again. If it \
                             still needs doing, enqueue a fresh item (ideally with what you \
                             learned about why it kept failing)."
                        )
                    } else {
                        format!("Released {id} back to the queue for another agent.")
                    }
                }
            })
        }
        "WorkList" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let url = format!("{}/agentmux/work", local_url.trim_end_matches('/'));
            let state = arguments.get("state").and_then(|v| v.as_str()).unwrap_or("");
            let limit = arguments.get("limit").and_then(|v| v.as_i64()).unwrap_or(50);
            // Built via reqwest's own query serializer, NOT string
            // concatenation (reagent P2 on PR #2902): `state` is a free-form
            // string in this tool's schema, not a validated enum, so a value
            // containing `&`, `#`, or `%` would otherwise corrupt the query
            // rather than being sent as the literal the caller intended.
            let resp = client
                .get(&url)
                .query(&[("state", state), ("limit", &limit.to_string())])
                .header(AUTH_KEY_HEADER, auth_key)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("work list request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("Tasks list failed: HTTP {status} — {text}");
            }
            let v: Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({}));
            let items = v.get("items").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            if items.is_empty() {
                return Ok("Queue is empty.".to_string());
            }
            let mut out = format!("{} work item(s):\n", items.len());
            for it in &items {
                let id = it.get("num").and_then(|x| x.as_i64()).map(|n| format!("#{n}"))
                    .unwrap_or_else(|| it.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string());
                let st = it.get("state").and_then(|x| x.as_str()).unwrap_or("");
                let title = it.get("title").and_then(|x| x.as_str()).unwrap_or("");
                let holder = it.get("claimed_by").and_then(|x| x.as_str()).unwrap_or("");
                let who = if holder.is_empty() { String::new() } else { format!(" [{holder}]") };
                out.push_str(&format!("  {id}  {st}{who}  {title}\n"));
                // `result` is the completion trace for a done item, and the
                // reason for a failed/released one — the very thing
                // WorkComplete calls "the only record". Omitting it here left
                // that record unreachable through the only agent-facing read
                // tool (Codex P2 on PR #2902).
                let result = it.get("result").and_then(|x| x.as_str()).unwrap_or("");
                if !result.is_empty() {
                    out.push_str(&format!("      -> {result}\n"));
                }
            }
            Ok(out)
        }
        _ => Err(not_in_family()),
    }
}

/// A task as an answer names it: `#12` when srv gave it a number, else its id.
fn task_name(v: &Value, id: &str) -> String {
    match v.get("num").and_then(|n| n.as_i64()) {
        Some(n) => format!("task #{n}"),
        None => format!("work item {id}"),
    }
}

/// The answer to finishing a task (`complete`, `submit`): where it went, and
/// the next task waiting for the caller, which it takes now (Kanban §6.5: an
/// agent goes on through its backlog).
fn finished_text(id: &str, v: &Value) -> String {
    let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("done");
    let mut out = if state == "review" {
        format!("Handed in task {id}: it waits for the operator's review in the Kanban.")
    } else {
        format!("Completed task {id}.")
    };
    match v.get("next").filter(|n| !n.is_null()) {
        Some(next) => {
            let num = next.get("num").and_then(|n| n.as_i64()).map(|n| format!("#{n}")).unwrap_or_default();
            let title = next.get("title").and_then(|t| t.as_str()).unwrap_or("");
            let who = match next.get("requested_by_kind").and_then(|k| k.as_str()) {
                Some("operator") => " (requested by the operator)".to_string(),
                Some("agent") => next
                    .get("requested_by")
                    .and_then(|r| r.as_str())
                    .filter(|r| !r.is_empty())
                    .map(|r| format!(" (requested by {r})"))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            let more = v.get("waiting").and_then(|w| w.as_i64()).unwrap_or(1) - 1;
            let after = if more > 0 { format!(" {more} more after it.") } else { String::new() };
            out.push_str(&format!(
                "\n\nNext in your backlog: {num} \"{title}\"{who}. Claim it with Tasks claim and start.{after}"
            ));
        }
        None => out.push_str(" Your backlog is empty."),
    }
    out
}
