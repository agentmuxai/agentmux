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

use agentmux_common::api_types::WpsPublishRequest;
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

/// How many `tool_chunk` events the broker keeps per `block:<id>`
/// scope. Sized for npm-install class output (~1000 lines is typical;
/// 1024 covers all but heavy CI builds). Each event is ~200–500 bytes,
/// so the steady-state ceiling is ~512 KB per actively-streaming block.
const TOOL_CHUNK_PERSIST: usize = 1024;

/// Event name for streaming bash chunks. Held constant across every
/// publish so the frontend can subscribe once per block on mount.
const TOOL_CHUNK_EVENT: &str = "tool_chunk";

/// Event name for the `PreCompact`-hook-sourced "compaction started"
/// signal. See `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
/// §4.2 / Tier 1.
const COMPACTION_STARTED_EVENT: &str = "compaction_started";

/// `persist: 0` — never retained/replayed. A late/reconnecting
/// subscriber must NOT see a past "compaction started" ping: there
/// is no matching completion tombstone (`compact_boundary` arrives
/// over the separate NDJSON stream, not WPS), so replaying a
/// retained start indistinguishably resurrects "Compacting…" for a
/// compaction that may have finished seconds after the ping — a
/// timestamp-age guard alone can't tell "recently finished" apart
/// from "still running" (Codex P1, PR #2378 round 2). Only a
/// currently-live subscriber should ever see this event; a
/// disconnected pane already clears its own `compacting` flag on
/// `StreamUnsubscribe` (reducer.ts), so there is nothing worth
/// replaying on reconnect regardless.
const COMPACTION_STARTED_PERSIST: usize = 0;

/// Fail-fast timeout for `publish_compaction_started` specifically —
/// much shorter than the 5s client-wide default used for `tool_chunk`
/// streaming (`from_env`). That default is fine for `exec`'s own async
/// runtime, but `precompact` is invoked as Claude Code's SYNCHRONOUS
/// `PreCompact` hook: the CLI blocks compaction on this process's exit,
/// so inheriting the 5s default means a wedged (but TCP-accepting)
/// backend delays the user's actual compaction by up to 5 real seconds —
/// directly contradicting this hook's own "never block/delay the
/// operation for observability" contract (see `precompact.rs`'s module
/// doc comment). Codex P2, PR #2378 round 4. 500ms comfortably covers a
/// normal same-host round trip while keeping the worst case imperceptible.
const COMPACTION_STARTED_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

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
        self.publish(TOOL_CHUNK_EVENT, TOOL_CHUNK_PERSIST, block_id, payload, None)
            .await
    }

    /// Publish a `compaction_started` event — the `PreCompact`-hook
    /// signal that fires the instant Claude Code begins compacting.
    /// `persist = 0`: live subscribers only, never replayed — see
    /// `COMPACTION_STARTED_PERSIST`'s doc comment. Uses
    /// `COMPACTION_STARTED_TIMEOUT` (much shorter than the client-wide
    /// default) since this call sits on a synchronous hook Claude Code
    /// blocks compaction on — see that constant's doc comment.
    pub async fn publish_compaction_started<T: Serialize>(
        &self,
        block_id: Option<&str>,
        payload: &T,
    ) -> Result<()> {
        self.publish(
            COMPACTION_STARTED_EVENT,
            COMPACTION_STARTED_PERSIST,
            block_id,
            payload,
            Some(COMPACTION_STARTED_TIMEOUT),
        )
        .await
    }

    /// Shared publish path for every WPS event this crate emits.
    /// `event`/`persist` vary per call site; the scoping (`block:<id>`)
    /// and auth/error-handling are identical across all of them.
    /// `timeout_override` replaces the client-wide default
    /// (`reqwest::RequestBuilder::timeout` overrides per-request) when
    /// `Some` — used by call sites sitting on a synchronous caller that
    /// must not inherit `from_env`'s more generous streaming timeout.
    async fn publish(
        &self,
        event: &str,
        persist: usize,
        block_id: Option<&str>,
        payload: &impl Serialize,
        timeout_override: Option<std::time::Duration>,
    ) -> Result<()> {
        let body = WpsPublishRequest {
            event: event.to_string(),
            scopes: block_id.map(|b| vec![format!("block:{b}")]).unwrap_or_default(),
            persist,
            data: serde_json::to_value(payload)?,
        };
        let url = format!("{}{}", self.endpoint, PUBLISH_PATH);
        let mut req = self
            .inner
            .post(&url)
            .header("X-AuthKey", &self.auth_key)
            .header("Content-Type", "application/json")
            .json(&body);
        if let Some(d) = timeout_override {
            req = req.timeout(d);
        }
        let resp = req
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
    use crate::test_env_lock::ENV_LOCK;

    // `std::env::set_var` / `remove_var` mutate process-global state;
    // cargo test runs tests in parallel by default, so without a
    // serial lock these races between threads (codex P1 on PR #804).
    // Shared with `precompact.rs`'s test module, which mutates the SAME
    // env vars — a private per-module lock doesn't actually synchronize
    // cross-module (Codex P2, PR #2378 round 6).
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

    // ── event/persist constants (refactor of `publish` into a shared
    // helper — SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.2) ──

    #[test]
    fn compaction_started_event_name_and_persist() {
        assert_eq!(COMPACTION_STARTED_EVENT, "compaction_started");
        // Never retained/replayed (Codex P1, PR #2378 round 2) — a
        // reconnecting subscriber must not resurrect a past ping for
        // a compaction that has since finished; see the constant's
        // doc comment for why an age-based guard alone can't fix
        // this on the receiving end.
        assert_eq!(COMPACTION_STARTED_PERSIST, 0);
    }

    #[test]
    fn compaction_started_uses_a_short_fail_fast_timeout() {
        // Codex P2, PR #2378 round 4: this publish sits on Claude Code's
        // SYNCHRONOUS PreCompact hook -- the CLI blocks compaction on this
        // process's exit, so it must not inherit the 5s client-wide
        // default used for tool_chunk streaming (from_env), or a wedged
        // backend delays the user's actual compaction by up to 5 real
        // seconds. Comfortably shorter than the client default, long
        // enough for a normal same-host round trip.
        assert!(COMPACTION_STARTED_TIMEOUT < std::time::Duration::from_secs(5));
        assert_eq!(COMPACTION_STARTED_TIMEOUT, std::time::Duration::from_millis(500));
    }

    #[test]
    fn tool_chunk_event_name_and_persist_unchanged_by_refactor() {
        // The `publish` extraction must not change `publish_chunk`'s
        // existing wire behavior.
        assert_eq!(TOOL_CHUNK_EVENT, "tool_chunk");
        assert_eq!(TOOL_CHUNK_PERSIST, 1024);
    }
}
