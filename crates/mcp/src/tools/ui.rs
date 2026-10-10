// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! UI automation and window capture.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    match name {
        "UIScreenshot" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let url = format!("{}/api/v1/ui/screenshot", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&UiScreenshotRequest { auth, pane: pane_arg(arguments) })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("screenshot failed: HTTP {status} — {text}");
            }
            let result: UiScreenshotResponse = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(format!(
                "Screenshot saved to {} — use Read on that path to view it yourself, or OpenMedia to show it to the user.",
                result.path
            ))
        }
        "CaptureWindow" => {
            // `index` stays an Option all the way into capture_window_impl —
            // NOT defaulted to 0 here. Defaulting it here is exactly what
            // let an ambiguous match silently capture the wrong (and once,
            // a genuinely unrelated/sensitive) window with no warning —
            // see docs/reports/REPORT_AGENT_SCREENSHOT_WINDOW_CONTROL_BLOCKERS_2026_08_24.md
            // §1. Losing "the caller didn't specify an index at all" vs.
            // "the caller explicitly asked for index 0" was the bug.
            let title_contains = arguments
                .get("title_contains")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let index = arguments
                .get("index")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let pid = arguments
                .get("pid")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);

            let query_desc = match (pid, title_contains.as_deref()) {
                (Some(p), _) => format!("pid={p}"),
                (None, Some(t)) => format!("title_contains={t:?}"),
                (None, None) => "<no target given>".to_string(),
            };

            // Audit trail, not a capability gate — reagent P1 (PR #2709
            // round 4): a real per-agent opt-in gate is a bigger feature
            // (settings storage + enforcement, no existing mechanism
            // anywhere in the codebase to build on — confirmed by the
            // spec this tool's own doc comment already cites) than fits
            // reactively on this PR, and this tool's actual residual risk
            // after rounds 2-3's scoping is disclosure across a human
            // boundary (a different AgentMux instance's window can belong
            // to a different OS user on a shared machine), not a technical
            // one — logging every call (who, what was requested, what
            // happened) is the honest, shippable Phase-1 answer while the
            // real gate is tracked separately (operator-confirmed).
            let mut resolved: Option<(CaptureTier, String)> = None;
            let outcome = capture_window_impl(title_contains.as_deref(), index, pid, &mut resolved);
            audit_log_capture_window(&query_desc, &outcome, &resolved);
            return outcome.map(|c| c.message);
        }
        "DiscoverWindows" => {
            let include_self = arguments
                .get("include_self")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // Non-AgentMux windows are enumerated now (they're T4 capture
            // targets) but stay OUT of the default listing: ordinary discovery
            // shouldn't disclose the titles of a user's unrelated applications
            // — their browser tabs, their password manager — as a side effect
            // of looking for AgentMux windows.
            let include_foreign = arguments
                .get("include_foreign")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let windows = enumerate_agentmux_windows()?;
            let list: Vec<Value> = windows
                .iter()
                // These two filters decide WHICH windows are listed. What each
                // listed entry may say about itself — in particular that a
                // withheld window is redacted rather than omitted — belongs to
                // `window_listing_entry` below, which documents that rationale
                // rather than repeating it here (reagentx P2 on PR #2845: an
                // earlier version of this comment still described a
                // `tier.allowed()` filter that has since been replaced, and
                // contradicted the code under it).
                .filter(|w| include_self || !w.is_self)
                .filter(|w| include_foreign || w.is_agentmux)
                .map(window_listing_entry)
                .collect();
            // reagent P1 on PR #2810: exe_path (embeds the OS username) for
            // OTHER instances/users on a shared machine is the same
            // disclosure-across-a-human-boundary risk CaptureWindow already
            // logs — this tool must too, not just the tool that follows it.
            audit_log_discover_windows(include_self, include_foreign, &list);
            return Ok(serde_json::to_string_pretty(&json!({ "windows": list }))
                .unwrap_or_else(|_| "{\"windows\":[]}".to_string()));
        }
        "UIClick" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let selector = arguments
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: selector"))?;
            let url = format!("{}/api/v1/ui/click", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&UiClickRequest {
                    auth,
                    pane: pane_arg(arguments),
                    selector: selector.to_string(),
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("click failed: HTTP {status} — {text}");
            }
            Ok(format!("Clicked {selector:?}"))
        }
        "UIQuery" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let selector = arguments
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: selector"))?;
            let limit = arguments.get("limit").and_then(|v| v.as_u64()).map(|n| n as u32);
            let url = format!("{}/api/v1/ui/query", local_url.trim_end_matches('/'));
            let resp = client
                .post(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .json(&UiQueryRequest {
                    auth,
                    pane: pane_arg(arguments),
                    selector: selector.to_string(),
                    limit,
                })
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("query failed: HTTP {status} — {text}");
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            let matches = body
                .get("data")
                .and_then(|d| d.get("matches"))
                .cloned()
                .unwrap_or_else(|| json!([]));
            Ok(serde_json::to_string_pretty(&matches).unwrap_or_else(|_| matches.to_string()))
        }
        "ListShortcuts" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let data = post_shortcuts(cx, "list", &UiShortcutsListRequest { auth }).await?;
            let list = data.get("shortcuts").cloned().unwrap_or_else(|| json!([]));
            Ok(serde_json::to_string_pretty(&list).unwrap_or_else(|_| list.to_string()))
        }
        "RunCommand" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let command = arguments
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: command"))?;
            let req = UiRunCommandRequest { auth, command: command.to_string(), target: str_arg(arguments, "target") };
            let data = post_shortcuts(cx, "run", &req).await?;
            if data.get("ran").and_then(|v| v.as_bool()) == Some(true) {
                Ok(format!("Ran {command}."))
            } else if let Some(pending) = data.get("pending") {
                // pane:close of another agent's pane: its user has 15 s to keep it.
                let why = data.get("reason").and_then(|v| v.as_str()).unwrap_or("waiting for the user");
                let id = pending.get("request_id").and_then(|v| v.as_str()).unwrap_or("?");
                Ok(format!("{command} is pending: {why} (request {id})."))
            } else {
                let why = data.get("reason").and_then(|v| v.as_str()).unwrap_or("no reason given");
                Ok(format!("Didn't run {command}: {why}"))
            }
        }
        "PressKeys" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let auth = sign_ui_automation_auth()?;
            let keys = arguments
                .get("keys")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: keys"))?;
            let req = UiPressKeysRequest { auth, keys: keys.to_string(), target: str_arg(arguments, "target") };
            let data = post_shortcuts(cx, "press", &req).await?;
            Ok(serde_json::to_string_pretty(&data).unwrap_or_else(|_| data.to_string()))
        }
        _ => Err(not_in_family()),
    }
}

fn str_arg(arguments: &Value, name: &str) -> Option<String> {
    arguments
        .get(name)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// POST to `/api/v1/ui/shortcuts/{route}` and return the response's `data`.
async fn post_shortcuts<T: serde::Serialize>(cx: &ToolCtx<'_>, route: &str, body: &T) -> Result<Value> {
    let url = format!("{}/api/v1/ui/shortcuts/{route}", cx.local_url.trim_end_matches('/'));
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() || body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = body.get("error").and_then(|v| v.as_str()).unwrap_or("no error message");
        anyhow::bail!("shortcuts/{route} failed: HTTP {status} — {err}");
    }
    Ok(body.get("data").cloned().unwrap_or(Value::Null))
}
