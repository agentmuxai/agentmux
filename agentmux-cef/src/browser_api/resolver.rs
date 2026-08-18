// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Resolve `block_id` → CDP target id.
//!
//! CEF's `/json` endpoint exposes every active page target but does
//! NOT expose the underlying `cef::Browser::identifier()`, so we can't
//! match by CEF id directly. Two resolution paths:
//!
//! **Path 1 — dedicated browser-pane block** (a `view: "browser"` widget,
//! embedding its own separate CEF `Browser` for arbitrary third-party
//! content): resolve by URL match, as originally shipped.
//! 1. Ask the pane manager for the pane's current URL.
//! 2. Probe `GET http://127.0.0.1:<debug>/json`.
//! 3. Find the entry whose `url` matches the pane's URL AND whose
//!    `id` isn't already owned by some other cached block.
//!
//! **Path 2 — any other block** (agent/terminal/editor/etc. panes, which
//! are DOM nodes inside a SHARED main/pool/floating window's page, not
//! their own CDP target — see `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`
//! §1): there is no URL-based way to disambiguate which window a given
//! block lives in, so instead probe every "page" target directly and ask
//! each one (via one `Runtime.evaluate`) whether its DOM contains a
//! `[data-blockid="<id>"]` element (set on every pane's root wrapper by
//! `frontend/app/block/blockframe.tsx`). First match wins.
//!
//! Callers MUST know which path resolved a target, because it changes
//! whether DOM-subtree scoping is required: a Path-2 target is a SHARED
//! page (other panes' DOM lives there too — queries/clicks/screenshots
//! must scope to `[data-blockid]`), while a Path-1 target already IS the
//! block's own isolated page (arbitrary third-party content with no
//! `data-blockid` concept — scoping would break it). See `ResolvedTarget`.
//!
//! Cache: `(block_id → ResolvedTarget)`. Known limit (Path 1, unchanged
//! from the original design): two browser panes navigated to the same URL
//! can't be distinguished. Phase-1 consumers use distinct URLs to avoid
//! the collision. A later phase can swap in a snapshot-at-create strategy
//! for bulletproof one-to-one mapping (see `SPEC_BROWSER_DOM_API.md` §5.5).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Deserialize;

use super::cdp::CdpSession;
use crate::state::AppState;

pub type ResolveError = String;

#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub target_id: String,
    /// True when this target is a page SHARED with other panes (the main
    /// AgentMux UI, or a pool/floating window) — callers must scope
    /// queries/clicks/screenshots to this block's own `[data-blockid]`
    /// subtree. False for a dedicated browser-pane target, which already
    /// IS this block's own page.
    pub scope_to_block: bool,
}

#[derive(Default)]
pub struct TargetCache {
    // block_id → resolved target
    entries: Mutex<HashMap<String, ResolvedTarget>>,
}

#[derive(Debug, Deserialize)]
struct JsonTarget {
    id: String,
    url: String,
    #[serde(default, rename = "type")]
    kind: String,
}

impl TargetCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn resolve(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
    ) -> Result<ResolvedTarget, ResolveError> {
        // Fast path: cache hit.
        if let Some(cached) = self.entries.lock().get(block_id).cloned() {
            return Ok(cached);
        }

        let debug_port = *state.debug_port.lock();
        if debug_port == 0 {
            return Err("CEF debug port not yet configured".to_string());
        }
        let already_cached = self.already_cached();

        // Path 1: a dedicated browser-pane block — resolve by URL match,
        // exactly as originally shipped.
        let resolved = if let Some(pane_url) = state.browser_panes.pane_url(state, block_id) {
            let target_id =
                Self::resolve_by_url(debug_port, &pane_url, block_id, &already_cached).await?;
            ResolvedTarget {
                target_id,
                scope_to_block: false,
            }
        } else {
            // Path 2 (fallback): not a browser pane — a DOM node inside
            // one of the shared window pages. Probe.
            let target_id =
                Self::resolve_by_dom_probe(debug_port, block_id, &already_cached).await?;
            ResolvedTarget {
                target_id,
                scope_to_block: true,
            }
        };

        self.entries
            .lock()
            .insert(block_id.to_string(), resolved.clone());
        Ok(resolved)
    }

    fn already_cached(&self) -> Vec<String> {
        self.entries
            .lock()
            .values()
            .map(|r| r.target_id.clone())
            .collect()
    }

    async fn fetch_page_targets(debug_port: u16) -> Result<Vec<JsonTarget>, ResolveError> {
        let json_url = format!("http://127.0.0.1:{debug_port}/json");
        let resp = reqwest::get(&json_url)
            .await
            .map_err(|e| format!("GET {json_url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("{json_url} returned {}", resp.status()));
        }
        resp.json()
            .await
            .map_err(|e| format!("parse /json: {e}"))
    }

    async fn resolve_by_url(
        debug_port: u16,
        pane_url: &str,
        block_id: &str,
        already_cached: &[String],
    ) -> Result<String, ResolveError> {
        let targets = Self::fetch_page_targets(debug_port).await?;

        // Filter:
        // - type=="page" (skip worker, iframe, etc.)
        // - url matches the pane's url (exact or trailing-slash-tolerant)
        // - id not already in our cache (avoids claiming another block's target)
        let pane_url_norm = normalize_url(pane_url);
        let candidate = targets
            .iter()
            .filter(|t| t.kind == "page" || t.kind.is_empty())
            .filter(|t| !already_cached.contains(&t.id))
            .find(|t| normalize_url(&t.url) == pane_url_norm);

        match candidate {
            Some(t) => Ok(t.id.clone()),
            None => Err(format!(
                "UNKNOWN_BLOCK_ID: no unclaimed CDP target matches url={pane_url} \
                 for block_id={block_id} (found {} targets)",
                targets.len()
            )),
        }
    }

    /// Probe every unclaimed "page" CDP target and ask each one (via a
    /// cheap `Runtime.evaluate`) whether its DOM contains this block's
    /// `[data-blockid]` wrapper. There are normally only a handful of live
    /// top-level windows, so this is fast; no window-topology bookkeeping
    /// (host↔srv round trip) is needed.
    async fn resolve_by_dom_probe(
        debug_port: u16,
        block_id: &str,
        already_cached: &[String],
    ) -> Result<String, ResolveError> {
        let targets = Self::fetch_page_targets(debug_port).await?;
        // JSON-encode block_id so it's a safe JS string literal; the
        // selector itself is built via CSS.escape inside the expression,
        // not string-concatenated here, so a block_id containing quotes
        // can't break out of the selector.
        let block_id_js = serde_json::to_string(block_id).unwrap_or_else(|_| "\"\"".to_string());
        let probe_expr = format!(
            "(() => {{ try {{ return !!document.querySelector(\
             '[data-blockid=\"' + CSS.escape({block_id_js}) + '\"]'); \
             }} catch (e) {{ return false; }} }})()"
        );

        for t in targets
            .iter()
            .filter(|t| (t.kind == "page" || t.kind.is_empty()) && !already_cached.contains(&t.id))
        {
            let ws_url = format!("ws://127.0.0.1:{debug_port}/devtools/page/{}", t.id);
            let mut cdp = match CdpSession::connect(&ws_url).await {
                Ok(c) => c,
                Err(_) => continue, // target may have closed between /json and connect
            };
            let reply = cdp
                .call(
                    "Runtime.evaluate",
                    serde_json::json!({ "expression": probe_expr, "returnByValue": true }),
                )
                .await;
            let _ = cdp.close().await;
            let found = reply
                .ok()
                .and_then(|v| v.get("result").and_then(|r| r.get("value")).cloned())
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if found {
                return Ok(t.id.clone());
            }
        }

        let probed = targets
            .iter()
            .filter(|t| (t.kind == "page" || t.kind.is_empty()) && !already_cached.contains(&t.id))
            .count();
        Err(format!(
            "UNKNOWN_BLOCK_ID: no CDP target's DOM contains a [data-blockid=\"{block_id}\"] \
             element (probed {probed} unclaimed page targets)"
        ))
    }

    /// Invalidate a block's cached target — called when a pane closes
    /// or navigates. On next resolve we'll re-probe.
    pub fn forget(&self, block_id: &str) {
        self.entries.lock().remove(block_id);
    }
}

fn normalize_url(u: &str) -> String {
    // `https://www.google.com` vs `https://www.google.com/` are the
    // same target; CEF /json includes the trailing slash, the pane's
    // meta.url in our tests usually doesn't.
    let trimmed = u.trim_end_matches('/').to_string();
    trimmed.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{normalize_url, TargetCache};

    #[test]
    fn normalize_strips_trailing_slash_and_lowercases() {
        assert_eq!(
            normalize_url("https://www.google.com/"),
            normalize_url("https://WWW.GOOGLE.COM"),
        );
    }

    /// Minimal fake CDP `/json` endpoint — serves `body` verbatim for every
    /// GET. Enough to exercise resolve_by_url / resolve_by_dom_probe's
    /// target-matching logic without a real CEF process.
    async fn spawn_fake_json_endpoint(body: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 2048];
                    let Ok(_) = stream.read(&mut buf).await else { return };
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        port
    }

    // Path 1 (dedicated browser-pane block): matches by normalized URL, and
    // — critically for the multi-pane case — must not re-claim a target
    // another cached block already owns even when the URL still matches.
    #[tokio::test]
    async fn resolve_by_url_matches_and_skips_already_claimed_targets() {
        let port = spawn_fake_json_endpoint(
            r#"[
                {"id": "target-a", "type": "page", "url": "https://example.com/pane-a/"},
                {"id": "target-b", "type": "page", "url": "https://example.com/pane-b"}
            ]"#,
        )
        .await;

        let found =
            TargetCache::resolve_by_url(port, "https://example.com/pane-a", "block-a", &[])
                .await
                .expect("should resolve target-a");
        assert_eq!(found, "target-a");

        let err = TargetCache::resolve_by_url(
            port,
            "https://example.com/pane-a",
            "block-a2",
            &["target-a".to_string()],
        )
        .await
        .expect_err("target-a is already claimed, must not be handed out twice");
        assert!(err.contains("UNKNOWN_BLOCK_ID"), "got: {err}");
    }

    // Path 2 (fallback — any other block): no real CDP server sits behind
    // these fake targets, so every connect attempt fails and this exercises
    // the "probed N targets, none matched" error path without needing to
    // fake the CDP WebSocket protocol itself.
    #[tokio::test]
    async fn resolve_by_dom_probe_reports_a_clear_error_when_nothing_matches() {
        let port = spawn_fake_json_endpoint(
            r#"[{"id": "target-a", "type": "page", "url": "https://example.com/"}]"#,
        )
        .await;

        let err = TargetCache::resolve_by_dom_probe(port, "some-block-id", &[])
            .await
            .expect_err("no CDP target is actually reachable in this test");
        assert!(err.contains("UNKNOWN_BLOCK_ID"), "got: {err}");
        assert!(err.contains("some-block-id"), "got: {err}");
    }
}
