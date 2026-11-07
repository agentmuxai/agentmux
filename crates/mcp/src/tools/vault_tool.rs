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

/// `history`: an entry's history, for an entry in a Vault this agent may use. No value in it.
async fn history(arguments: &Value, cx: &ToolCtx<'_>, base: &str) -> Result<String> {
    let id = arguments.get("id").and_then(Value::as_str).unwrap_or("").trim();
    if id.is_empty() {
        anyhow::bail!("Vault history needs an id: the entry whose history to read (list shows them)");
    }
    let body = json!({
        "id": id,
        "limit": arguments.get("limit").and_then(Value::as_i64),
        "auth": crate::identity::sign_ui_automation_auth().map_err(|e| anyhow::anyhow!("Vault history needs this agent's signed identity: {e}"))?,
    });
    let result = crate::srv_http::srv_post_json(cx.client, &format!("{base}/api/v1/vault/history"), cx.auth_key, &body, "Vault history").await?;
    Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
}

/// `import`: copy a secret from AWS Secrets Manager into the Vault. The person is asked once; the agent gets ids and names.
async fn import(arguments: &Value, cx: &ToolCtx<'_>, base: &str) -> Result<String> {
    let source = arguments.get("source").cloned().unwrap_or(Value::Null);
    if !source.get("aws_secret_id").is_some_and(Value::is_string) {
        anyhow::bail!("Vault import needs source.aws_secret_id: the AWS secret to copy (its name or ARN)");
    }
    let s = |k: &str| arguments.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let wait = arguments.get("wait_secs").and_then(Value::as_u64).unwrap_or(REQUEST_WAIT_SECS).min(REQUEST_WAIT_MAX_SECS);
    let body = json!({
        "source": source,
        "kind": s("kind"),
        "label": s("label"),
        "id": arguments.get("id").and_then(Value::as_str),
        "fields": arguments.get("fields").cloned().unwrap_or_else(|| json!({})),
        "vault": arguments.get("vault").and_then(Value::as_str),
        "reason": arguments.get("reason").and_then(Value::as_str),
        "refs": arguments.get("refs").cloned().unwrap_or_else(|| json!([])),
        "wait_secs": wait,
        "auth": crate::identity::sign_ui_automation_auth().map_err(|e| anyhow::anyhow!("Vault import needs this agent's signed identity: {e}"))?,
    });
    // The person's answer, then AWS: the wait plus the CLI's 25 s and some room.
    let url = format!("{base}/api/v1/vault/import");
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .timeout(std::time::Duration::from_secs(wait + 90))
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!(crate::srv_http::describe_transport_error("Vault import", &url, &e)))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(crate::srv_http::describe_http_error("Vault import", status, &text));
    }
    let result: Value = serde_json::from_str(&text).unwrap_or_default();
    Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    match name {
        "Vault" => {
            if cx.local_url.is_empty() || cx.auth_key.is_empty() {
                anyhow::bail!("Vault: AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. Is this agent pane opened via AgentMux?");
            }
            let base = cx.local_url.trim_end_matches('/');
            match arguments.get("action").and_then(Value::as_str).unwrap_or("") {
                "list" => {
                    // Which entries it sees depends on which Vaults the person made open to this agent.
                    let body = json!({ "auth": crate::identity::sign_ui_automation_auth().ok() });
                    let result = crate::srv_http::srv_post_json(cx.client, &format!("{base}/api/v1/vault/list"), cx.auth_key, &body, "Vault list").await?;
                    Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
                }
                "request" => {
                    let s = |k: &str| arguments.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let wait = arguments.get("wait_secs").and_then(Value::as_u64).unwrap_or(REQUEST_WAIT_SECS).min(REQUEST_WAIT_MAX_SECS);
                    let body = json!({
                        "kind": s("kind"),
                        "label": s("label"),
                        "why": s("why"),
                        "reason": arguments.get("reason").and_then(Value::as_str),
                        "vault": arguments.get("vault").and_then(Value::as_str),
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
                        "reason": arguments.get("reason").and_then(Value::as_str),
                        "fields": arguments.get("fields").cloned().unwrap_or_else(|| json!({})),
                        "rules": arguments.get("rules").cloned().unwrap_or_else(|| json!({})),
                        "expires": arguments.get("expires").and_then(Value::as_str),
                        "vault": arguments.get("vault").and_then(Value::as_str),
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
                        "reason": arguments.get("reason").and_then(Value::as_str),
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
                "history" => history(arguments, cx, base).await,
                "import" => import(arguments, cx, base).await,
                other => anyhow::bail!("Vault: unknown action {other:?}; use list, request, generate, rotate, history or import"),
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
        assert_eq!(actions, &json!(["list", "request", "generate", "rotate", "history", "import"]));
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
