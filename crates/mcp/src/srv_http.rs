// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP helpers for calls to the sidecar srv.
//! Split out of main.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.2).

use super::*;

/// Guard for the self-scoped agent-API verbs: they need the sidecar URL +
/// auth key to reach it, and the caller's block id to resolve "my own"
/// tab/pane/window/workspace.
pub(crate) fn require_agent_env(local_url: &str, auth_key: &str, block_id: &str) -> Result<()> {
    if local_url.is_empty() || auth_key.is_empty() {
        anyhow::bail!(
            "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set. \
             Is this agent pane opened via AgentMux?"
        );
    }
    if block_id.is_empty() {
        anyhow::bail!(
            "neither AGENTMUX_AGENT_BUS_ID nor AGENTMUX_BLOCKID is set \
             — cannot resolve this agent's context. Is this agent pane opened via AgentMux?"
        );
    }
    Ok(())
}

/// GET a srv route and return its JSON body, with every failure described by
/// what actually happened (#3473). reqwest's own message for a refused
/// connection or a timeout is only "error sending request for url (…)", which
/// reads like a network fault whatever the cause; and parsing the body before
/// checking the status loses the status whenever an error body isn't JSON.
/// So: refused / timed out / other transport failures are told apart, the
/// status is checked first, and an error carries srv's own `error` message,
/// else the raw body.
pub(crate) async fn srv_get_json(
    client: &reqwest::Client,
    url: &str,
    auth_key: &str,
    query: &[(&str, String)],
    what: &str,
) -> Result<Value> {
    let resp = client
        .get(url)
        .header(AUTH_KEY_HEADER, auth_key)
        .query(query)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!(describe_transport_error(what, url, &e)))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("{what}: reading AgentMux's response failed: {e}"))?;
    if !status.is_success() {
        anyhow::bail!(describe_http_error(what, status, &text));
    }
    serde_json::from_str(&text).map_err(|e| {
        anyhow::anyhow!("{what}: AgentMux answered HTTP {status} but not with JSON ({e})")
    })
}

pub(crate) fn describe_transport_error(what: &str, url: &str, e: &reqwest::Error) -> String {
    let mut causes = String::new();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        causes.push_str(&format!(": {s}"));
        source = s.source();
    }
    if e.is_timeout() {
        format!(
            "{what} timed out: AgentMux didn't answer in time. This is not an empty result — retry, \
             or ask for less"
        )
    } else if e.is_connect() {
        format!(
            "{what}: can't reach AgentMux at {url}{causes}. Is it still running? An agent started \
             by an AgentMux that has since restarted must be reopened."
        )
    } else {
        format!("{what}: the request to AgentMux failed: {e}{causes}")
    }
}

pub(crate) fn describe_http_error(what: &str, status: reqwest::StatusCode, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| {
            let body = body.trim();
            if body.is_empty() {
                "(no body)".to_string()
            } else {
                body.chars().take(300).collect()
            }
        });
    let hint = match status.as_u16() {
        401 => {
            " — AgentMux rejected this process's auth key (AGENTMUX_AUTH_KEY). The agent was \
             probably started by an AgentMux that has since restarted; reopen it."
        }
        503 => " — temporary; retry shortly.",
        _ => "",
    };
    format!("{what} failed (HTTP {status}): {detail}{hint}")
}

/// How long a Shell or PtyShell create may take: one on an SSH host the user
/// has not allowed yet waits for their answer in a consent dialog (srv gives
/// them two minutes), well past the client's usual 10 s.
pub(crate) const SHELL_CREATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(150);
