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
        .map_err(|e| proxy_error(route, &e))?;
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

/// What the agent reads when the host did not answer a browser call. A call that ran out of time says so, and
/// what to try: a page still on its way is the usual reason (#47890).
fn proxy_error(route: &str, e: &reqwest::Error) -> String {
    proxy_error_text(route, e.is_timeout(), &e.to_string())
}

fn proxy_error_text(route: &str, timed_out: bool, why: &str) -> String {
    if timed_out {
        format!("proxy to host {route}: the page did not answer in time ({why}); it may still be loading: Browser wait with until_state load, then try again")
    } else {
        format!("proxy to host {route}: {why}")
    }
}

#[cfg(test)]
mod tests {
    use super::proxy_error_text;

    #[test]
    fn a_call_that_ran_out_of_time_says_the_page_may_still_be_loading() {
        let m = proxy_error_text("act", true, "operation timed out");
        assert!(m.starts_with("proxy to host act: the page did not answer in time") && m.contains("until_state load"), "{m}");
    }

    #[test]
    fn any_other_failure_keeps_its_own_words() {
        assert_eq!(proxy_error_text("act", false, "connection refused"), "proxy to host act: connection refused");
    }
}
