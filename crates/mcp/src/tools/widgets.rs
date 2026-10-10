// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Widgets: list them, install one the user then approves, open its pane
//! (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §10).

use super::*;

/// How long `WidgetInstall` waits for the user by default, and at most.
const INSTALL_WAIT_SECS: u64 = 300;
const INSTALL_WAIT_MAX_SECS: u64 = 600;

fn require_srv(local_url: &str, auth_key: &str) -> Result<()> {
    if local_url.is_empty() || auth_key.is_empty() {
        anyhow::bail!("AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. Is this agent pane opened via AgentMux?");
    }
    Ok(())
}

async fn list(cx: &ToolCtx<'_>) -> Result<Vec<Value>> {
    let url = format!("{}/api/v1/widgets", cx.local_url.trim_end_matches('/'));
    let resp = cx.client.get(&url).header(AUTH_KEY_HEADER, cx.auth_key).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        anyhow::bail!("listing widgets failed: HTTP {status}: {}", resp.text().await.unwrap_or_default());
    }
    let body: Value = resp.json().await.map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
    Ok(body.get("packages").and_then(Value::as_array).cloned().unwrap_or_default())
}

/// A path the agent gave, made absolute against its own workspace.
fn resolve_path(path: &str) -> String {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        return path.to_string();
    }
    let base = std::env::var("AGENTMUX_AGENT_WORKDIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    base.join(p).to_string_lossy().into_owned()
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "WidgetList" => {
            require_srv(cx.local_url, cx.auth_key)?;
            let rows: Vec<Value> = list(cx)
                .await?
                .iter()
                .map(|p| {
                    json!({
                        "id": p["id"], "name": p["name"], "version": p["version"], "kind": p["kind"], "state": p["state"],
                        "permissions": p["permissions"], "error": p["error"], "folder": p["folder"],
                        "views": p["panes"].as_array().map(|ps| ps.iter().map(|x| x["view"].clone()).collect::<Vec<_>>()).unwrap_or_default(),
                    })
                })
                .collect();
            Ok(serde_json::to_string_pretty(&json!({ "widgets": rows })).unwrap_or_default())
        }
        "WidgetInstall" => {
            require_srv(cx.local_url, cx.auth_key)?;
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: path"))?;
            let wait = arguments.get("wait_secs").and_then(Value::as_u64).unwrap_or(INSTALL_WAIT_SECS).min(INSTALL_WAIT_MAX_SECS);
            // The prompt names this agent only from its signed identity; an
            // agent with no signing key yet is shown as unverified.
            let body = json!({
                "path": resolve_path(path),
                "replace": arguments.get("replace").and_then(Value::as_bool).unwrap_or(true),
                "auth": crate::identity::sign_ui_automation_auth().ok(),
                "wait_secs": wait,
            });
            let url = format!("{}/api/v1/widgets/install", cx.local_url.trim_end_matches('/'));
            let resp = cx
                .client
                .post(&url)
                .header(AUTH_KEY_HEADER, cx.auth_key)
                .timeout(std::time::Duration::from_secs(wait + 60))
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            let status = resp.status();
            let result: Value = resp.json().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("the widget wasn't installed: {}", result["error"].as_str().unwrap_or("unknown error"));
            }
            Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        "OpenWidget" => {
            require_srv(cx.local_url, cx.auth_key)?;
            let view = arguments
                .get("view")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: view"))?;
            let packages = list(cx).await?;
            let found = packages.iter().find_map(|p| {
                p["panes"].as_array().and_then(|ps| ps.iter().find(|x| x["view"] == view)).map(|pane| (p, pane))
            });
            let Some((pkg, pane)) = found else {
                anyhow::bail!("no installed widget has the view {view}; WidgetList shows the views there are");
            };
            if pkg["state"] != "approved" {
                anyhow::bail!(
                    "{} is {}, so it can't open; the user approves or turns it on in Settings → Widgets",
                    pkg["id"].as_str().unwrap_or(""),
                    pkg["state"].as_str().unwrap_or("not approved")
                );
            }
            // The pane's defaults, then the agent's own keys, all in the
            // widget's namespace (`widget:<id>:<key>`).
            let id = pkg["id"].as_str().unwrap_or_default();
            let mut meta = pane["default_meta"].as_object().cloned().unwrap_or_default();
            if let Some(own) = arguments.get("meta").and_then(Value::as_object) {
                for (k, v) in own {
                    meta.insert(format!("widget:{id}:{k}"), v.clone());
                }
            }
            meta.insert("view".into(), json!(view));
            let split = arguments.get("split").and_then(Value::as_str).filter(|s| matches!(*s, "right" | "down")).unwrap_or("right");
            let mut req = json!({ "view": view, "meta": meta, "focus": true });
            if !cx.block_id.is_empty() {
                req["split_direction"] = json!(split);
                req["split_reference_block_id"] = json!(cx.block_id);
            }
            let url = format!("{}/api/v1/pane/open", cx.local_url.trim_end_matches('/'));
            let resp = cx
                .client
                .post(&url)
                .header(AUTH_KEY_HEADER, cx.auth_key)
                .json(&req)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            let status = resp.status();
            let result: Value = resp.json().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!("the pane didn't open: HTTP {status}: {result}");
            }
            Ok(format!("Opened {view} (pane {}).", result["block_id"].as_str().unwrap_or("?")))
        }
        _ => Err(not_in_family()),
    }
}
