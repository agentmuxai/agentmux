// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! UI automation and window capture.

use super::*;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "UIScreenshot" => run_ui_screenshot(arguments, cx).await,
        "UIRecord" => run_ui_record(arguments, cx).await,
        "CaptureWindow" => run_capture_window(arguments, cx).await,
        "DiscoverWindows" => run_discover_windows(arguments, cx).await,
        "UIClick" => run_ui_click(arguments, cx).await,
        "UIQuery" => run_ui_query(arguments, cx).await,
        "ListShortcuts" => run_list_shortcuts(arguments, cx).await,
        "RunCommand" => run_run_command(arguments, cx).await,
        "PressKeys" => run_press_keys(arguments, cx).await,
        "SetShortcut" => run_set_shortcut(arguments, cx).await,
        "ResetShortcuts" => run_reset_shortcuts(arguments, cx).await,
        _ => Err(not_in_family()),
    }
}

async fn run_ui_screenshot(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
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
    screenshot_text(&result, crate::self_keys::in_container(), &container_media_dir())
}

/// What `UIScreenshot` tells the agent. srv wrote the PNG on its own machine, so a container agent, which cannot read
/// that path, gets its own copy in the container (from the bytes srv sent along) and is told that path instead: the
/// one its file tools read and `files` on Tasks and Message can pass on.
fn screenshot_text(result: &UiScreenshotResponse, in_container: bool, dir: &std::path::Path) -> Result<String> {
    if !in_container {
        return Ok(format!(
            "Screenshot saved to {} — use Read on that path to view it yourself, or Pane (action \"open\", view \"media\") to show it to the user.",
            result.path
        ));
    }
    let local = save_local_copy(dir, &result.png_base64, "png")?;
    Ok(format!(
        "Screenshot saved to {local} (inside your container) — use Read on that path to view it yourself, or pass it in `files` of Tasks update or submit, or of Message send."
    ))
}

/// Where a container agent's copies of screenshots and recordings go: its own temp folder.
fn container_media_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("agentmux-media")
}

/// Write `base64` (a file srv sent inline) to a new `<dir>/<id>.<ext>` and return its path. Files older than an hour
/// that were written here go too, so the folder does not only grow.
fn save_local_copy(dir: &std::path::Path, base64: &str, ext: &str) -> Result<String> {
    use base64::Engine as _;
    if base64.is_empty() {
        anyhow::bail!("srv sent no image with the {ext}, so there is nothing to save in your container: the file is on the host and cannot be read from here");
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .map_err(|e| anyhow::anyhow!("the {ext} srv sent could not be decoded: {e}"))?;
    std::fs::create_dir_all(dir).map_err(|e| anyhow::anyhow!("cannot make {}: {e}", dir.display()))?;
    prune_old_media(dir);
    let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&path, bytes).map_err(|e| anyhow::anyhow!("cannot save {}: {e}", path.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Delete the PNGs and GIFs in `dir` that are older than the capture retention. Best effort.
fn prune_old_media(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if !matches!(path.extension().and_then(|e| e.to_str()), Some("png" | "gif")) {
            continue;
        }
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > crate::window_capture::CAPTURE_RETENTION);
        if old {
            let _ = std::fs::remove_file(&path);
        }
    }
}

async fn run_ui_record(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let auth = sign_ui_automation_auth()?;
    let op = arguments.get("op").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if op != "start" && op != "stop" {
        anyhow::bail!("record needs op: \"start\" to begin a recording, \"stop\" to end it and get the file");
    }
    let number = |key: &str| arguments.get(key).and_then(|v| v.as_u64()).map(|v| v.min(u64::from(u32::MAX)) as u32);
    let url = format!("{}/api/v1/ui/record", local_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header(AUTH_KEY_HEADER, auth_key)
        .json(&UiRecordRequest {
            auth,
            op,
            pane: pane_arg(arguments),
            fps: number("fps"),
            seconds: number("seconds"),
            inline: crate::self_keys::in_container(),
        })
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("record failed: HTTP {status} — {text}");
    }
    let result: UiRecordResponse = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
    record_text(&result, crate::self_keys::in_container(), &container_media_dir())
}

/// What `UIRecord` tells the agent. A finished recording is on srv's machine; a container agent gets its own copy
/// (srv sends the bytes when asked) and the message names that path instead of the host's.
fn record_text(result: &UiRecordResponse, in_container: bool, dir: &std::path::Path) -> Result<String> {
    let (true, Some(host_path)) = (in_container, result.path.as_deref()) else {
        return Ok(result.message.clone());
    };
    let local = save_local_copy(dir, result.gif_base64.as_deref().unwrap_or_default(), "gif")?;
    Ok(result.message.replace(host_path, &format!("{local} (inside your container)")))
}

async fn run_capture_window(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
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
    let outcome = capture_blacked_out(title_contains.as_deref(), index, pid, &mut resolved, cx).await;
    audit_log_capture_window(&query_desc, &outcome, &resolved);
    outcome.map(|c| c.message)
}

/// The capture, with an AgentMux window's private and secret panes blacked out (`window_blackout`): srv asks the
/// windows for their rectangles just before the frame and just after, and anything that doesn't match refuses it.
async fn capture_blacked_out(
    title_contains: Option<&str>,
    index: Option<usize>,
    pid: Option<u32>,
    resolved: &mut Option<(CaptureTier, String)>,
    cx: &ToolCtx<'_>,
) -> Result<CaptureOutcome> {
    use crate::window_blackout as wb;
    if crate::self_keys::in_container() {
        anyhow::bail!(wb::IN_CONTAINER);
    }
    let target = resolve_window(title_contains, index, pid, resolved)?;
    if !target.is_agentmux {
        let (image, unrendered) = capture_frame(&target)?;
        return finish_capture(&target, &image, unrendered, false);
    }
    if !target.is_self {
        anyhow::bail!(wb::OTHER_INSTANCE);
    }
    let capture_id = uuid::Uuid::new_v4().to_string();
    let begins = capture_masks(cx, &capture_id, "begin").await?;
    let begin = wb::before(&begins, &target.title)?.clone();
    let (mut image, unrendered) = capture_frame(&target)?;
    let ends = capture_masks(cx, &capture_id, "end").await?;
    wb::after(&begin, &ends, image.width(), image.height())?;
    wb::black_out(&mut image, &begin);
    finish_capture(&target, &image, unrendered, true)
}

/// Every window's answer to one ask of srv's `capture_mask`.
async fn capture_masks(cx: &ToolCtx<'_>, capture_id: &str, phase: &str) -> Result<Vec<crate::window_blackout::WindowMask>> {
    let ToolCtx { local_url, auth_key, client, .. } = *cx;
    if local_url.is_empty() || auth_key.is_empty() {
        anyhow::bail!("AgentMux windows can't be captured without srv, which gives the rectangles of their private panes");
    }
    let url = format!("{}/api/v1/ui/capture_mask", local_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header(AUTH_KEY_HEADER, auth_key)
        .json(&json!({ "capture_id": capture_id, "phase": phase }))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("the rectangles of the window's private panes could not be read, so it is not captured: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        anyhow::bail!("{}", body.get("error").and_then(Value::as_str).unwrap_or("the rectangles of the window's private panes could not be read, so it is not captured"));
    }
    serde_json::from_value(body.get("masks").cloned().unwrap_or_else(|| json!([])))
        .map_err(|e| anyhow::anyhow!("srv's answer for the window's private panes was not understood, so it is not captured: {e}"))
}

async fn run_discover_windows(arguments: &Value, _cx: &ToolCtx<'_>) -> Result<String> {
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

async fn run_ui_click(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
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

async fn run_ui_query(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
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

async fn run_list_shortcuts(_arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let auth = sign_ui_automation_auth()?;
    let data = post_shortcuts(cx, "list", &UiShortcutsListRequest { auth }).await?;
    let list = data.get("shortcuts").cloned().unwrap_or_else(|| json!([]));
    Ok(serde_json::to_string_pretty(&list).unwrap_or_else(|_| list.to_string()))
}

async fn run_run_command(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, .. } = *cx;
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
        // pane:close of another agent's pane: its user has 30 s to keep it (10 s for a browser pane).
        let why = data.get("reason").and_then(|v| v.as_str()).unwrap_or("waiting for the user");
        let id = pending.get("request_id").and_then(|v| v.as_str()).unwrap_or("?");
        Ok(format!("{command} is pending: {why} (request {id})."))
    } else {
        let why = data.get("reason").and_then(|v| v.as_str()).unwrap_or("no reason given");
        Ok(format!("Didn't run {command}: {why}"))
    }
}

async fn run_press_keys(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, .. } = *cx;
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

/// `bind`: give a command exactly these keys. The page refuses what an agent may not do and says what is in the way.
async fn run_set_shortcut(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let auth = sign_ui_automation_auth()?;
    let command = str_arg(arguments, "command").ok_or_else(|| anyhow::anyhow!("missing required parameter: command"))?;
    let bindings = match arguments.get("bindings") {
        Some(v) => v
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("`bindings` must be a list of keys (an empty list unbinds the command)"))?
            .iter()
            .map(|k| k.as_str().map(str::to_string).ok_or_else(|| anyhow::anyhow!("`bindings` must be a list of strings")))
            .collect::<Result<Vec<_>>>()?,
        None => return Err(anyhow::anyhow!("missing required parameter: bindings (a list of keys; empty to unbind the command)")),
    };
    let req = UiSetShortcutRequest { auth, command, bindings, when: str_arg(arguments, "when") };
    let data = post_shortcuts(cx, "set", &req).await?;
    Ok(shortcut_edit_said(&data))
}

/// `reset`: one command, or every command, back on its defaults.
async fn run_reset_shortcuts(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let auth = sign_ui_automation_auth()?;
    let all = arguments.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let req = UiResetShortcutsRequest { auth, command: str_arg(arguments, "command"), all };
    let data = post_shortcuts(cx, "reset", &req).await?;
    Ok(shortcut_edit_said(&data))
}

/// What a change to the person's shortcuts says: what changed, or why not and what is in the way.
fn shortcut_edit_said(data: &Value) -> String {
    if data.get("changed").and_then(|v| v.as_bool()) == Some(true) {
        let summary = data.get("summary").and_then(|v| v.as_str()).unwrap_or("changed");
        let note = data.get("note").and_then(|v| v.as_str()).unwrap_or("");
        return format!("Changed: {summary}. {note}").trim_end().to_string();
    }
    let why = data.get("reason").and_then(|v| v.as_str()).unwrap_or("no reason given");
    format!("Not changed: {why}")
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

#[cfg(test)]
mod call_tests;
