// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser-pane DOM API (`/agentmux/browser/*`).
//!
//! External clients (test harnesses, automation scripts) use these
//! endpoints to query and mutate the DOM inside a running browser
//! pane without depending on screen-pixel geometry. Implemented as a
//! thin proxy over CEF's Chrome DevTools Protocol (CDP) server on
//! `remote_debugging_port` (9223 dev / 9222 release).
//!
//! Phase 1 implements only `browser.query` — a CSS-selector lookup
//! that returns matching elements with their tag, text, attrs, and
//! bounding rect. Subsequent phases layer on `focus_info`, `eval`,
//! `screenshot`, and the write methods (`click_element`,
//! `dispatch_key`, …). See `docs/specs/SPEC_BROWSER_DOM_API.md` and
//! `docs/specs/PLAN_BROWSER_DOM_API.md`.
//!
//! All routes require `Authorization: Bearer <ipc_token>` — same
//! scheme as the existing `/ipc` route in `crate::ipc`.

use std::sync::Arc;

use axum::routing::post;
use axum::Router;

use crate::state::AppState;

pub mod cdp;
pub mod resolver;
pub mod routes;
pub mod types;

/// Register `/agentmux/browser/*` routes on the given axum Router.
/// Called from `ipc::start_ipc_server` alongside the existing
/// `/ipc` + `/health` routes.
pub fn register_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route("/agentmux/browser/query", post(routes::query))
}

/// Shared state for the browser API — primarily the CDP target
/// cache (block_id → target_id). Lives inside `AppState` and is
/// lazily populated on first resolve per block.
pub struct BrowserApiState {
    pub target_cache: resolver::TargetCache,
}

impl BrowserApiState {
    pub fn new() -> Self {
        Self {
            target_cache: resolver::TargetCache::new(),
        }
    }
}

impl Default for BrowserApiState {
    fn default() -> Self {
        Self::new()
    }
}
