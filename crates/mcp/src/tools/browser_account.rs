// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser` action `generate_password`: the Vault makes the password of an
//! account the agent signs up for, and types it, once the person says yes
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §7).

use super::*;
use crate::identity::sign_ui_automation_auth;
use crate::srv_http::require_agent_env;

/// `generate_password`: the Vault makes the password of a new account, once the person says yes.
pub(super) async fn run(arguments: &Value, cx: &ToolCtx<'_>) -> Result<String> {
    let ToolCtx { local_url, auth_key, block_id, .. } = *cx;
    require_agent_env(local_url, auth_key, block_id)?;
    let auth = sign_ui_automation_auth()?;
    let text = |k: &str| arguments.get(k).and_then(Value::as_str);
    let ref_ = text("ref").map(str::trim).filter(|r| !r.is_empty());
    let Some(ref_) = ref_ else {
        anyhow::bail!("generate_password needs ref: the first password field of the sign-up form, from a snapshot or a find");
    };
    let body = json!({ "auth": auth, "pane": pane_arg(arguments), "ref": ref_, "username": text("username"), "reason": text("reason") });
    let url = format!("{}/api/v1/ui/browser/generate_password", cx.local_url.trim_end_matches('/'));
    // The person is asked first, and then the pane counts down.
    let resp = cx
        .client
        .post(&url)
        .header(AUTH_KEY_HEADER, cx.auth_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(11 * 60 + 30))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("request failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let why = body.get("error").and_then(Value::as_str).map(str::to_string).unwrap_or(text);
        anyhow::bail!("Browser generate_password: {why}");
    }
    Ok(generate_password_text(&body))
}

/// The agent's answer: which entry holds the password and for which site, never the value.
fn generate_password_text(body: &Value) -> String {
    let d = |k: &str| body.pointer(&format!("/data/{k}")).and_then(Value::as_str).unwrap_or("");
    format!(
        "Browser generate_password: the Vault made a password for a new account on {} and typed it into the new-password field and its confirmation field (hidden: not shown to you, in snapshots or in read-backs). It is kept as the Vault entry {} until the form is sent. Fill the other fields yourself and click the form's button, then take a snapshot to see whether the site accepted it; if it rejected the password, call generate_password again.{}",
        d("site"),
        d("entry"),
        super::browser::approval_text(body.pointer("/data/approval"))
    )
}
