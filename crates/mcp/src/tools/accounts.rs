// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity accounts. Not advertised: `Identity` (identity_tool.rs) calls
//! these by their old names.

use super::*;

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// The remote operation and params for `accounts` or `switch` with a `host`.
fn remote_request(name: &str, arguments: &Value) -> Result<(&'static str, Value)> {
    match name {
        "IdentityAccounts" => Ok(("identity.accounts", json!({}))),
        _ => {
            let account = str_arg(arguments, "account").ok_or_else(|| anyhow::anyhow!("missing required parameter: account"))?;
            let agent = str_arg(arguments, "agent")
                .ok_or_else(|| anyhow::anyhow!("with a host, name the agent on that machine (or \"all limited\"): there is no \"yourself\" across machines"))?;
            let mut params = json!({ "agent_id": agent, "account": account });
            if let Some(when) = str_arg(arguments, "when") {
                params["when"] = json!(when);
            }
            Ok(("identity.switch", params))
        }
    }
}

/// `accounts` or `switch` on another machine, from its own answer.
async fn remote(name: &str, arguments: &Value, cx: &ToolCtx<'_>, host: &str) -> Result<String> {
    let (op, params) = remote_request(name, arguments)?;
    let reason = str_arg(arguments, "reason").unwrap_or_else(|| format!("Identity {}", op.trim_start_matches("identity.")));
    let answer = remote_op::remote_op_call(cx, host, op, params, &reason).await?;
    match remote_op::explain_not_done(host, op, &answer) {
        Some(problem) => anyhow::bail!("{problem}"),
        None => Ok(remote_op::pretty(host, &answer)),
    }
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    if matches!(name, "IdentityAccounts" | "IdentitySwitch") {
        if let Some(host) = remote_op::host_arg(arguments) {
            return remote(name, arguments, cx, &host).await;
        }
    }
    match name {
        "IdentityAccounts" => {
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/identity/accounts", local_url.trim_end_matches('/'));
            let resp = client
                .get(&url)
                .header(AUTH_KEY_HEADER, auth_key)
                .query(&[("agent_id", agent_id.as_str()), ("scope", arguments.get("scope").and_then(|v| v.as_str()).unwrap_or(""))])
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("identity/accounts failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "IdentityValidate" => {
            let account_id = arguments
                .get("account_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: account_id"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/identity/validate", local_url.trim_end_matches('/'));
            let body = json!({
                "agent_id": agent_id,
                "account_id": account_id,
            });
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
                anyhow::bail!("identity/validate failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        "IdentitySwitch" => {
            let account = arguments
                .get("account")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing required parameter: account"))?;
            require_agent_env(local_url, auth_key, block_id)?;
            let agent_id = agent_slug()?;
            let url = format!("{}/api/v1/agent/identity/switch", local_url.trim_end_matches('/'));
            let mut body = json!({ "agent_id": agent_id, "account": account });
            if let Some(target) = arguments.get("agent").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                body["target"] = json!(target);
            }
            if let Some(when) = arguments.get("when").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                body["when"] = json!(when);
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
                anyhow::bail!("identity/switch failed: HTTP {status} — {text}");
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
            Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()))
        }
        _ => Err(not_in_family()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_on_a_host_asks_for_the_listing() {
        let (op, params) = remote_request("IdentityAccounts", &json!({ "host": "charlie" })).unwrap();
        assert_eq!((op, params), ("identity.accounts", json!({})));
    }

    #[test]
    fn a_switch_on_a_host_names_the_agent_and_the_account() {
        let args = json!({ "host": "charlie", "agent": "Opaz", "account": "any with quota", "when": "now" });
        let (op, params) = remote_request("IdentitySwitch", &args).unwrap();
        assert_eq!(op, "identity.switch");
        assert_eq!(params, json!({ "agent_id": "Opaz", "account": "any with quota", "when": "now" }));
        let e = remote_request("IdentitySwitch", &json!({ "host": "charlie", "account": "x" })).unwrap_err().to_string();
        assert!(e.contains("name the agent"), "{e}");
        assert!(remote_request("IdentitySwitch", &json!({ "host": "charlie", "agent": "Opaz" })).is_err());
    }
}
