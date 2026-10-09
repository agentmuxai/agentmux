// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Browser-pane DOM API (`/agentmux/browser/*`).
//!
//! External clients (test harnesses, automation scripts) use these
//! endpoints to query and mutate the DOM inside a running browser
//! pane without depending on screen-pixel geometry. Implemented as a
//! thin proxy over the Chrome DevTools Protocol (CDP), driven in-process
//! through each CEF `Browser`'s own DevTools message channel (`cdp`) —
//! no remote-debugging server needed, which is what lets release builds
//! run without one (#3681, `crate::cdp_port`).
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

pub mod act;
pub mod cdp;
pub mod resolver;
pub mod routes;
pub mod snapshot;
pub mod types;

/// Register `/agentmux/browser/*` routes on the given axum Router.
/// Called from `ipc::start_ipc_server` alongside the existing
/// `/ipc` + `/health` routes.
pub fn register_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/agentmux/browser/query", post(routes::query))
        .route("/agentmux/browser/focus_info", post(routes::focus_info))
        .route("/agentmux/browser/eval", post(routes::eval))
        .route("/agentmux/browser/snapshot", post(act::snapshot_route))
        .route("/agentmux/browser/act", post(act::act_route))
        .route("/agentmux/browser/set_files", post(act::set_files_route))
        .route("/agentmux/browser/wait_for", post(act::wait_for_route))
        .route("/agentmux/browser/screenshot", post(routes::screenshot))
        .route("/agentmux/browser/click_element", post(routes::click_element))
        .route("/agentmux/browser/focus_element", post(routes::focus_element))
        .route("/agentmux/browser/dispatch_key", post(routes::dispatch_key))
        .route("/agentmux/browser/navigate", post(routes::navigate))
        .route("/agentmux/browser/back", post(routes::back))
        .route("/agentmux/browser/forward", post(routes::forward))
        .route("/agentmux/browser/reload", post(routes::reload))
        .route("/agentmux/browser/owned_panes", post(routes::owned_panes))
}

/// Shared state for the browser API — primarily the resolver's
/// cache (block_id → window label, for panes inside shared windows). Lives inside `AppState` and is
/// lazily populated on first resolve per block.
pub struct BrowserApiState {
    pub target_cache: resolver::TargetCache,
    /// Each pane's latest snapshot references (`act::RefTable`).
    pub ref_tables: act::RefTables,
}

impl BrowserApiState {
    pub fn new() -> Self {
        Self {
            target_cache: resolver::TargetCache::new(),
            ref_tables: act::RefTables::default(),
        }
    }
}

impl Default for BrowserApiState {
    fn default() -> Self {
        Self::new()
    }
}
