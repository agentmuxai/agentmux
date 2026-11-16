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

/// Posts `body` to a Vault route that may wait for the person for `wait` seconds before it answers.
async fn post_waiting(cx: &ToolCtx<'_>, url: &str, body: &Value, what: &str, wait: u64) -> Result<String> {
    let resp = cx
        .client
        .post(url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .timeout(std::time::Duration::from_secs(wait + 90))
        .json(body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!(crate::srv_http::describe_transport_error(what, url, &e)))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(crate::srv_http::describe_http_error(what, status, &text));
    }
    let result: Value = serde_json::from_str(&text).unwrap_or_default();
    Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
}

/// What an entry says of its key beside the place: who reads it, the date it last changed, where else it is kept.
const SAID_OF_THE_KEY: [&str; 3] = ["readers", "rotated", "copies"];

/// Whether an `update` changes where the entry's machine copy is (a `source_*` field) or what the entry says of its
/// key: the person may be asked first, so the call waits as long as the answer may take.
fn changes_machine_copy(arguments: &Value) -> bool {
    arguments.get("fields").and_then(Value::as_object).is_some_and(|f| f.keys().any(|k| k.starts_with("source_") || SAID_OF_THE_KEY.contains(&k.as_str())))
}

/// `check`: whether an entry's machine copy still holds the entry's value. The answer is `same`, `different` or
/// `could not read` with why; neither value comes back.
async fn check(arguments: &Value, cx: &ToolCtx<'_>, base: &str) -> Result<String> {
    let id = arguments.get("id").and_then(Value::as_str).unwrap_or("").trim();
    if id.is_empty() {
        anyhow::bail!("Vault check needs an id: the entry whose machine copy to check (list shows which have a source)");
    }
    let body = json!({
        "id": id,
        "reason": arguments.get("reason").and_then(Value::as_str),
        "auth": crate::identity::sign_ui_automation_auth().map_err(|e| anyhow::anyhow!("Vault check needs this agent's signed identity: {e}"))?,
    });
    // The read of the other place: the CLI's 25 s and some room.
    post_waiting(cx, &format!("{base}/api/v1/vault/check"), &body, "Vault check", 0).await
}

/// `show`, `verify` and `health` read the Vault; `update` changes an entry's details. None returns a value.
async fn inspect(action: &str, arguments: &Value, cx: &ToolCtx<'_>, base: &str) -> Result<String> {
    let text = |k: &str| arguments.get(k).and_then(Value::as_str);
    let auth = crate::identity::sign_ui_automation_auth();
    let wait = arguments.get("wait_secs").and_then(Value::as_u64).unwrap_or(REQUEST_WAIT_SECS).min(REQUEST_WAIT_MAX_SECS);
    let body = match action {
        "show" | "verify" => {
            if action == "show" && text("id").is_none_or(|i| i.trim().is_empty()) {
                anyhow::bail!("Vault show needs an id: the entry to look at (list shows them)");
            }
            json!({ "id": text("id"), "auth": auth.ok() })
        }
        "update" => json!({
            "id": text("id").filter(|i| !i.trim().is_empty()).ok_or_else(|| anyhow::anyhow!("Vault update needs an id: the entry to change (list shows them)"))?,
            "label": text("label"),
            "fields": arguments.get("fields").filter(|f| f.is_object()).cloned().unwrap_or_else(|| json!({})),
            "tags": arguments.get("tags").cloned(),
            "reason": text("reason"),
            "wait_secs": wait,
            "auth": auth.map_err(|e| anyhow::anyhow!("Vault update needs this agent's signed identity: {e}"))?,
        }),
        _ => json!({ "auth": auth.ok() }),
    };
    let what = format!("Vault {action}");
    if action == "update" && changes_machine_copy(arguments) {
        return post_waiting(cx, &format!("{base}/api/v1/vault/update"), &body, &what, wait).await;
    }
    let result = crate::srv_http::srv_post_json(cx.client, &format!("{base}/api/v1/vault/{action}"), cx.auth_key, &body, &what).await?;
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

/// `rotate`: give an entry a new value the Vault makes, and set it at its service with the agent's command.
async fn rotate(arguments: &Value, cx: &ToolCtx<'_>, base: &str) -> Result<String> {
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

/// How long srv lets a batch's reads take, after the person's answer.
const BATCH_READ_SECS: u64 = 300;

/// `import_batch`: copy a named list of secrets from AWS Secrets Manager into one Vault under one approval. The
/// person sees a row for every secret and may leave rows out; the agent gets a row per secret, never a value.
async fn import_batch(arguments: &Value, cx: &ToolCtx<'_>, base: &str) -> Result<String> {
    if !arguments.get("items").and_then(Value::as_array).is_some_and(|items| !items.is_empty()) {
        anyhow::bail!("Vault import_batch needs items: the secrets to copy, each with aws_secret_id, kind and label (or id, to replace an entry's value)");
    }
    let wait = arguments.get("wait_secs").and_then(Value::as_u64).unwrap_or(REQUEST_WAIT_SECS).min(REQUEST_WAIT_MAX_SECS);
    let body = json!({
        "batch": arguments.get("batch").and_then(Value::as_str).unwrap_or(""),
        "vault": arguments.get("vault").and_then(Value::as_str),
        "reason": arguments.get("reason").and_then(Value::as_str),
        "source": arguments.get("source").filter(|s| s.is_object()).cloned().unwrap_or_else(|| json!({})),
        "items": arguments.get("items").cloned().unwrap_or_else(|| json!([])),
        "refs": arguments.get("refs").cloned().unwrap_or_else(|| json!([])),
        "wait_secs": wait,
        "auth": crate::identity::sign_ui_automation_auth().map_err(|e| anyhow::anyhow!("Vault import_batch needs this agent's signed identity: {e}"))?,
    });
    // The person's answer, then every read in turn.
    post_waiting(cx, &format!("{base}/api/v1/vault/import_batch"), &body, "Vault import_batch", wait + BATCH_READ_SECS).await
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
                "rotate" => rotate(arguments, cx, base).await,
                "history" => history(arguments, cx, base).await,
                "import" => import(arguments, cx, base).await,
                "import_batch" => import_batch(arguments, cx, base).await,
                "check" => check(arguments, cx, base).await,
                action @ ("show" | "update" | "verify" | "health") => inspect(action, arguments, cx, base).await,
                other => anyhow::bail!("Vault: unknown action {other:?}; use list, request, generate, rotate, history, import, import_batch, check, show, update, verify or health"),
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
        assert_eq!(actions, &json!(["list", "request", "generate", "rotate", "history", "import", "import_batch", "check", "show", "update", "verify", "health"]));
        // srv's catalog (backend/vault/kinds.rs) is the source; this list
        // must name the same kinds.
        let srv = include_str!("../../../srv-core/src/backend/vault/kinds.rs");
        let mut srv_kinds: Vec<&str> = srv.split("name: \"").skip(1).filter_map(|rest| rest.split('"').next()).collect();
        let mut ours: Vec<&str> = schema["inputSchema"]["properties"]["kind"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        srv_kinds.sort_unstable();
        ours.sort_unstable();
        assert_eq!(ours, srv_kinds);
    }

    #[test]
    fn only_an_update_of_the_machine_copy_waits_for_the_person() {
        assert!(changes_machine_copy(&json!({ "fields": { "source_kind": "aws-secret", "source_location": "staging/app" } })));
        assert!(changes_machine_copy(&json!({ "fields": { "notes": "x", "source_key": "" } })));
        // A linked entry's readers, date and other places are the person's to allow too.
        for said in ["readers", "rotated", "copies"] {
            assert!(changes_machine_copy(&json!({ "fields": { said: "x" } })), "{said}");
        }
        assert!(!changes_machine_copy(&json!({ "fields": { "service": "Example", "notes": "the source of truth" } })));
        assert!(!changes_machine_copy(&json!({ "label": "Example" })));
    }
}
