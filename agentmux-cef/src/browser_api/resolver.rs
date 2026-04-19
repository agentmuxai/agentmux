// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Resolve `block_id` → CDP target id.
//!
//! CEF's `/json` endpoint exposes every active page target but does
//! NOT expose the underlying `cef::Browser::identifier()`, so we can't
//! match by CEF id directly. Phase 1 strategy:
//!
//! 1. Ask the pane manager for the pane's current URL.
//! 2. Probe `GET http://127.0.0.1:<debug>/json`.
//! 3. Find the entry whose `url` matches the pane's URL AND whose
//!    `id` isn't already owned by some other cached block.
//! 4. Cache `(block_id → target_id)` for subsequent calls.
//!
//! Known limit: two panes navigated to the same URL can't be
//! distinguished this way. Phase-1 consumers (dom-smoke, stress
//! harness) use distinct URLs to avoid the collision. A later phase
//! can swap in a snapshot-at-create strategy for bulletproof
//! one-to-one mapping (see `SPEC_BROWSER_DOM_API.md` §5.5).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Deserialize;

use crate::state::AppState;

pub type ResolveError = String;

#[derive(Default)]
pub struct TargetCache {
    // block_id → target_id
    entries: Mutex<HashMap<String, String>>,
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
    ) -> Result<String, ResolveError> {
        // Fast path: cache hit.
        if let Some(cached) = self.entries.lock().get(block_id).cloned() {
            return Ok(cached);
        }

        // Determine the pane's current URL on the CEF UI side. The
        // browser_panes manager exposes this via main_frame().url()
        // when the pane is Live.
        let pane_url = state
            .browser_panes
            .pane_url(state, block_id)
            .ok_or_else(|| {
                format!("UNKNOWN_BLOCK_ID: no live browser pane for block_id={block_id}")
            })?;

        // Probe /json.
        let debug_port = *state.debug_port.lock();
        if debug_port == 0 {
            return Err("CEF debug port not yet configured".to_string());
        }
        let json_url = format!("http://127.0.0.1:{debug_port}/json");
        let resp = reqwest::get(&json_url)
            .await
            .map_err(|e| format!("GET {json_url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("{json_url} returned {}", resp.status()));
        }
        let targets: Vec<JsonTarget> = resp
            .json()
            .await
            .map_err(|e| format!("parse /json: {e}"))?;

        // Filter:
        // - type=="page" (skip worker, iframe, etc.)
        // - url matches the pane's url (exact or trailing-slash-tolerant)
        // - id not already in our cache (avoids claiming another block's target)
        let already_cached: Vec<String> = self
            .entries
            .lock()
            .values()
            .cloned()
            .collect();

        let pane_url_norm = normalize_url(&pane_url);
        let candidate = targets
            .iter()
            .filter(|t| t.kind == "page" || t.kind.is_empty())
            .filter(|t| !already_cached.contains(&t.id))
            .find(|t| normalize_url(&t.url) == pane_url_norm);

        let target_id = match candidate {
            Some(t) => t.id.clone(),
            None => {
                return Err(format!(
                    "UNKNOWN_BLOCK_ID: no unclaimed CDP target matches url={pane_url} \
                     for block_id={block_id} (found {} targets)",
                    targets.len()
                ))
            }
        };

        self.entries
            .lock()
            .insert(block_id.to_string(), target_id.clone());
        Ok(target_id)
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
    use super::normalize_url;

    #[test]
    fn normalize_strips_trailing_slash_and_lowercases() {
        assert_eq!(
            normalize_url("https://www.google.com/"),
            normalize_url("https://WWW.GOOGLE.COM"),
        );
    }
}
