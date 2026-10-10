// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! POST to the paired CEF host's browser API (`/agentmux/browser/*`).

use axum::http::StatusCode;

use super::HostIpc;

/// `ui_handlers::proxy_to_host_timeout` for a caller that holds the HTTP client rather
/// than the whole `AppState` (Tower's sampler, `app_api/tower_pane.rs`).
pub(crate) async fn post_to_host(
    http_client: &reqwest::Client,
    host: &HostIpc,
    route: &str,
    body: serde_json::Value,
    timeout: Option<std::time::Duration>,
) -> Result<serde_json::Value, String> {
    let url = format!("http://127.0.0.1:{}/agentmux/browser/{route}", host.port);
    let mut request = http_client
        .post(&url)
        .header("Authorization", format!("Bearer {}", host.token))
        .json(&body);
    if let Some(t) = timeout {
        request = request.timeout(t);
    }
    let resp = request
        .send()
        .await
        .map_err(|e| format!("proxy to host {route}: {e}"))?;
    if resp.status() == StatusCode::UNAUTHORIZED {
        return Err(
            "host rejected our ipc_token — stale registration after a host restart? \
             will self-heal on the host's next host_ipc.Register call"
                .to_string(),
        );
    }
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("parse host {route} response: {e}"))
}
