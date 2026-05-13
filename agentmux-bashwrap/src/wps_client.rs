// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Thin HTTP client for publishing WPS events to the AgentMux sidecar.
//!
//! Reads `AGENTMUX_LOCAL_URL` and `AGENTMUX_AUTH_KEY` from the
//! process environment (inherited from the agentmux-srv → claude →
//! bash subprocess chain). Attaches `X-AuthKey` per PR #801 to
//! authenticate against the auth_middleware-gated publish endpoint.
//!
//! Designed to silently degrade: if env vars are absent or the
//! sidecar is unreachable, publishes fail-fast and the caller emits
//! a `kind: "system"` chunk that surfaces the degradation to the
//! user without aborting the command itself.

use anyhow::Result;
use serde::Serialize;
use std::sync::Arc;

const PUBLISH_PATH: &str = "/agentmux/wps/publish";

/// Per-launch sidecar config + reusable HTTP client. Cheap to clone
/// (the inner `reqwest::Client` uses Arc internally).
#[derive(Clone)]
pub struct WpsClient {
    inner: Arc<reqwest::Client>,
    endpoint: String,
    auth_key: String,
}

#[derive(Serialize)]
struct PublishRequest<'a, T: Serialize> {
    /// WPS event name. Fixed `tool_chunk` for every streaming chunk —
    /// the tool_use_id lives in the payload, not in the event name.
    /// Lets the frontend open a single per-block subscription on mount
    /// instead of per-tool subscriptions racing the tool's execution.
    event: &'a str,
    /// Scope filters. Always `["block:<id>"]` for tool_chunk so the
    /// broker delivers only to the relevant pane.
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    scopes: &'a [String],
    /// Per-event persistence count (kept by the broker for
    /// replay-on-subscribe). Captures the wrapper's full output for
    /// late subscribers when Claude buffers the tool_use stream until
    /// after the tool runs.
    #[serde(skip_serializing_if = "is_zero_usize")]
    persist: usize,
    /// Event payload. Free-form per event type.
    data: &'a T,
}

fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}

/// How many `tool_chunk` events the broker keeps per `block:<id>`
/// scope. Sized for npm-install class output (~1000 lines is typical;
/// 1024 covers all but heavy CI builds). Each event is ~200–500 bytes,
/// so the steady-state ceiling is ~512 KB per actively-streaming block.
const TOOL_CHUNK_PERSIST: usize = 1024;

/// Event name for streaming bash chunks. Held constant across every
/// publish so the frontend can subscribe once per block on mount.
const TOOL_CHUNK_EVENT: &str = "tool_chunk";

impl WpsClient {
    /// Build a client from the env. Returns `None` when the required
    /// env vars are missing — the caller surfaces a system chunk
    /// rather than crashing.
    pub fn from_env() -> Option<Self> {
        let endpoint = std::env::var("AGENTMUX_LOCAL_URL").ok()?;
        let auth_key = std::env::var("AGENTMUX_AUTH_KEY").ok()?;
        if endpoint.is_empty() || auth_key.is_empty() {
            return None;
        }
        let inner = reqwest::Client::builder()
            // Snappy publish — chunks should land within a frame of
            // emission. Long timeouts here cause back-pressure that
            // stalls the PTY reader.
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .ok()?;
        Some(Self {
            inner: Arc::new(inner),
            endpoint: endpoint.trim_end_matches('/').to_string(),
            auth_key,
        })
    }

    /// Publish a `tool_chunk` event. The tool_use_id lives in the
    /// payload (not on this method's signature) so a single per-block
    /// subscription receives chunks for every tool in that block.
    /// Persists with `TOOL_CHUNK_PERSIST` so late subscribers (the
    /// frontend, which learns about the tool_use only after Claude
    /// buffers the stream) get the full history on subscribe.
    pub async fn publish_chunk<T: Serialize>(
        &self,
        block_id: Option<&str>,
        payload: &T,
    ) -> Result<()> {
        let scopes: Vec<String> = block_id
            .map(|b| vec![format!("block:{}", b)])
            .unwrap_or_default();
        let body = PublishRequest {
            event: TOOL_CHUNK_EVENT,
            scopes: &scopes,
            persist: TOOL_CHUNK_PERSIST,
            data: payload,
        };
        let url = format!("{}{}", self.endpoint, PUBLISH_PATH);
        let resp = self
            .inner
            .post(&url)
            .header("X-AuthKey", &self.auth_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("publish failed: HTTP {} — {}", status, text);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `std::env::set_var` / `remove_var` mutate process-global state;
    // cargo test runs tests in parallel by default, so without a
    // serial lock these races between threads (codex P1 on PR #804).
    // A single mutex covers every test in this module that touches
    // the env.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        std::env::remove_var("AGENTMUX_LOCAL_URL");
        std::env::remove_var("AGENTMUX_AUTH_KEY");
    }

    #[test]
    fn from_env_returns_none_without_endpoint() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        assert!(WpsClient::from_env().is_none());
    }

    #[test]
    fn from_env_returns_none_when_only_endpoint_set() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("AGENTMUX_LOCAL_URL", "http://127.0.0.1:1");
        assert!(WpsClient::from_env().is_none());
        clear_env();
    }

    #[test]
    fn from_env_strips_trailing_slash() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_env();
        std::env::set_var("AGENTMUX_LOCAL_URL", "http://127.0.0.1:9999/");
        std::env::set_var("AGENTMUX_AUTH_KEY", "x");
        let c = WpsClient::from_env().expect("client");
        assert_eq!(c.endpoint, "http://127.0.0.1:9999");
        clear_env();
    }
}
