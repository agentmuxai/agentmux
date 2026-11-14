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

pub mod accessible_name;
pub mod act;
pub mod capture;
pub mod capture_download;
pub mod cdp;
pub mod chooser_files;
pub mod click_delivery;
pub mod clickable;
pub mod combobox;
pub mod dates;
pub mod file_chooser;
pub mod frame_scope;
pub mod frames;
pub mod hibernation;
pub mod hidden_choices;
mod hidden_secrets;
pub mod key_typing;
pub mod last_input;
pub mod nav_settle;
mod orphan_labels;
pub mod page_state;
pub mod page_url;
pub mod press;
pub mod ref_tables;
pub mod resolver;
pub mod routes;
pub mod secret;
mod secret_fill;
pub mod select_lists;
pub mod shortcuts;
pub mod shown_once;
pub mod signin_fill;
pub mod signup_fill;
pub mod signup_probe;
pub mod snapshot;
pub mod terminal;
pub mod text_read;
pub mod types;
pub mod visibility;
pub mod choice_facts;
pub mod vault_card_form;
pub mod vault_fill;
pub mod vault_slots;
pub mod wait;
pub mod zoom;

/// Register `/agentmux/browser/*` routes on the given axum Router.
/// Called from `ipc::start_ipc_server` alongside the existing
/// `/ipc` + `/health` routes.
pub fn register_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/agentmux/browser/query", post(routes::query))
        .route("/agentmux/browser/focus_info", post(routes::focus_info))
        .route("/agentmux/browser/eval", post(routes::eval))
        .route("/agentmux/browser/page_url", post(page_url::page_url_route))
        .route("/agentmux/browser/last_input", post(last_input::last_input_route))
        .route("/agentmux/browser/snapshot", post(act::snapshot_route))
        .route("/agentmux/browser/find", post(act::find_route))
        .route("/agentmux/browser/act", post(act::act_route))
        // Uploads carry the files' bytes (base64 in JSON): up to 25 MB of
        // files, so well past axum's 2 MiB default body limit.
        .route(
            "/agentmux/browser/set_files",
            post(act::set_files_route).layer(act::set_files_body_limit()),
        )
        .route("/agentmux/browser/wait_for", post(wait::wait_for_route))
        .route("/agentmux/browser/field_info", post(vault_fill::field_info_route))
        .route("/agentmux/browser/vault_fill", post(vault_fill::vault_fill_route))
        .route("/agentmux/browser/card_form", post(vault_card_form::card_form_route))
        .route("/agentmux/browser/signin_fill", post(signin_fill::signin_fill_route))
        .route("/agentmux/browser/signup_fill", post(signup_fill::signup_fill_route))
        .route("/agentmux/browser/signup_probe", post(signup_probe::signup_probe_route))
        .route("/agentmux/browser/capture_value", post(capture::capture_value_route))
        .route("/agentmux/browser/capture_mark", post(capture::capture_mark_route))
        .route("/agentmux/browser/capture_download", post(capture_download::capture_download_route))
        .route("/agentmux/browser/screenshot", post(routes::screenshot))
        .route("/agentmux/browser/click_element", post(routes::click_element))
        .route("/agentmux/browser/focus_element", post(routes::focus_element))
        .route("/agentmux/browser/dispatch_key", post(routes::dispatch_key))
        .route("/agentmux/browser/press", post(press::press_route))
        .route("/agentmux/browser/chooser_files", post(chooser_files::chooser_files_route).layer(act::set_files_body_limit()))
        .route("/agentmux/browser/terminal_state", post(terminal::terminal_state))
        .route("/agentmux/browser/terminal_keys", post(terminal::terminal_keys))
        .route("/agentmux/browser/navigate", post(routes::navigate))
        .route("/agentmux/browser/back", post(routes::back))
        .route("/agentmux/browser/forward", post(routes::forward))
        .route("/agentmux/browser/reload", post(routes::reload))
        .route("/agentmux/browser/zoom", post(zoom::zoom_route))
        .route("/agentmux/browser/owned_panes", post(routes::owned_panes))
        .route("/agentmux/browser/dialog_answer", post(routes::dialog_answer))
        .route("/agentmux/browser/renderer_map", post(routes::renderer_map))
        .route("/agentmux/browser/list_shortcuts", post(shortcuts::list_shortcuts))
        .route("/agentmux/browser/run_command", post(shortcuts::run_command))
        .route("/agentmux/browser/press_keys", post(shortcuts::press_keys))
        .route("/agentmux/browser/set_shortcut", post(shortcuts::set_shortcut))
        .route("/agentmux/browser/reset_shortcuts", post(shortcuts::reset_shortcuts))
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
