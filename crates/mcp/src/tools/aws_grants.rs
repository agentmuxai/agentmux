// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Identity `aws_grants` and `aws_grant`: the AWS grants this machine shares with its container agents, and the answer
//! to a grant that is new or wider than what was allowed. Not advertised by name: `Identity` (identity_tool.rs) calls
//! these.

use super::*;

fn arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

pub(super) async fn call(name: &str, arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, client, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let base = local_url.trim_end_matches('/');
    let request = match name {
        "IdentityAwsGrants" => client.get(format!("{base}/api/v1/aws/grants")),
        "IdentityAwsGrant" => {
            let need = |key: &str| arg(arguments, key).ok_or_else(|| anyhow::anyhow!("missing required parameter: {key}"));
            let body = json!({ "grant": need("grant")?, "change": need("change")?, "decision": need("decision")?, "reason": need("reason")? });
            client.post(format!("{base}/api/v1/aws/grants/decide")).json(&body)
        }
        _ => return Err(not_in_family()),
    };
    let resp = request.header(AUTH_KEY_HEADER, auth_key).send().await.map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let said = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string)).unwrap_or(text);
        anyhow::bail!("aws grants: HTTP {status}: {said}");
    }
    let result: Value = serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("response parse failed: {e}"))?;
    Ok(pretty(&result))
}
