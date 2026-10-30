// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Vault: the person's secrets, named but never read
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.4).
//! `list` reads entries' names; `request` asks the person for a new entry
//! and waits for the answer; `generate` creates an entry whose value the
//! Vault makes (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.1).
//! No route an agent can call returns a value.

use super::*;

/// How long `request` waits for the person by default, and at most.
const REQUEST_WAIT_SECS: u64 = 300;
const REQUEST_WAIT_MAX_SECS: u64 = 600;

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Vault" => {
            if cx.local_url.is_empty() || cx.auth_key.is_empty() {
                anyhow::bail!("Vault: AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. Is this agent pane opened via AgentMux?");
            }
            let base = cx.local_url.trim_end_matches('/');
            match arguments.get("action").and_then(Value::as_str).unwrap_or("") {
                "list" => {
                    let body = crate::srv_http::srv_get_json(cx.client, &format!("{base}/api/v1/vault"), cx.auth_key, &[], "Vault list").await?;
                    Ok(serde_json::to_string_pretty(&body).unwrap_or_default())
                }
                "request" => {
                    let s = |k: &str| arguments.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let wait = arguments.get("wait_secs").and_then(Value::as_u64).unwrap_or(REQUEST_WAIT_SECS).min(REQUEST_WAIT_MAX_SECS);
                    let body = json!({
                        "kind": s("kind"),
                        "label": s("label"),
                        "why": s("why"),
                        "auth": crate::identity::sign_ui_automation_auth().ok(),
                        "wait_secs": wait,
                    });
                    let url = format!("{base}/api/v1/vault/request");
                    let resp = cx
                        .client
                        .post(&url)
                        .header(AUTH_KEY_HEADER, cx.auth_key)
                        .timeout(std::time::Duration::from_secs(wait + 60))
                        .json(&body)
                        .send()
                        .await
                        .map_err(|e| anyhow::anyhow!(crate::srv_http::describe_transport_error("Vault request", &url, &e)))?;
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if !status.is_success() {
                        anyhow::bail!(crate::srv_http::describe_http_error("Vault request", status, &text));
                    }
                    let result: Value = serde_json::from_str(&text).unwrap_or_default();
                    Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
                }
                "generate" => {
                    let s = |k: &str| arguments.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let body = json!({
                        "kind": s("kind"),
                        "label": s("label"),
                        "why": s("why"),
                        "fields": arguments.get("fields").cloned().unwrap_or_else(|| json!({})),
                        "rules": arguments.get("rules").cloned().unwrap_or_else(|| json!({})),
                        "expires": arguments.get("expires").and_then(Value::as_str),
                        "auth": crate::identity::sign_ui_automation_auth().map_err(|e| anyhow::anyhow!("Vault generate needs this agent's signed identity: {e}"))?,
                    });
                    let result = crate::srv_http::srv_post_json(cx.client, &format!("{base}/api/v1/vault/generate"), cx.auth_key, &body, "Vault generate").await?;
                    Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
                }
                "rotate" => {
                    let s = |k: &str| arguments.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let body = json!({
                        "id": s("id"),
                        "set": s("set"),
                        "verify": arguments.get("verify").and_then(Value::as_str),
                        "cwd": arguments.get("cwd").and_then(Value::as_str),
                        "rules": arguments.get("rules").cloned().unwrap_or_else(|| json!({})),
                        "auth": crate::identity::sign_ui_automation_auth().map_err(|e| anyhow::anyhow!("Vault rotate needs this agent's signed identity: {e}"))?,
                    });
                    // The person's approval, then set and verify, each up to 10 minutes.
                    let url = format!("{base}/api/v1/vault/rotate");
                    let resp = cx
                        .client
                        .post(&url)
                        .header(AUTH_KEY_HEADER, cx.auth_key)
                        .timeout(std::time::Duration::from_secs(32 * 60))
                        .json(&body)
                        .send()
                        .await
                        .map_err(|e| anyhow::anyhow!(crate::srv_http::describe_transport_error("Vault rotate", &url, &e)))?;
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if !status.is_success() {
                        anyhow::bail!(crate::srv_http::describe_http_error("Vault rotate", status, &text));
                    }
                    let result: Value = serde_json::from_str(&text).unwrap_or_default();
                    Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
                }
                other => anyhow::bail!("Vault: unknown action {other:?}; use list, request, generate or rotate"),
            }
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_lists_every_action_and_every_kind_srv_knows() {
        let schema: Value = serde_json::from_str(crate::tool_schemas::VAULT_TOOL).unwrap();
        let actions = &schema["inputSchema"]["properties"]["action"]["enum"];
        assert_eq!(actions, &json!(["list", "request", "generate", "rotate"]));
        // srv's catalog (backend/vault/kinds.rs) is the source; this list
        // must name the same kinds.
        let srv = include_str!("../../../srv/src/backend/vault/kinds.rs");
        let mut srv_kinds: Vec<&str> = srv.split("name: \"").skip(1).filter_map(|rest| rest.split('"').next()).collect();
        let mut ours: Vec<&str> = schema["inputSchema"]["properties"]["kind"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        srv_kinds.sort_unstable();
        ours.sort_unstable();
        assert_eq!(ours, srv_kinds);
    }
}
