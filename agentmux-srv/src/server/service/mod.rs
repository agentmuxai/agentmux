// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP `/agentmux/service` dispatcher, split into concern-based submodules.
//!
//! * [`object`] / [`client`] / [`window`] / [`workspace`] / [`misc`] — the
//!   per-service RPC handlers `dispatch_service` routes to.
//! * [`introspect`] — agent-facing read-only tree snapshots + `AgentContext`
//!   resolution (backs `/api/v1/self`, `/layout`, naming verbs).
//! * [`object_helpers`] — wave-object read/update/meta primitives.
//! * [`reducer_helpers`] — reducer dispatch / event publish / compensation.
//! * [`layout_helpers`] — layout-tree + pending-action writers used by the
//!   drag-and-drop / tear-off / redock handlers.
//!
//! `pub use` re-exports keep every external call site
//! (`crate::server::service::…`, `super::service::…`) unchanged.

mod client;
mod credential;
mod host_ipc;
mod introspect;
pub(crate) mod layout_helpers;
mod misc;
mod object;
mod object_helpers;
mod reducer_helpers;
pub(crate) mod session_restore;
mod tab_lifecycle;
mod tab_move;
mod tear_off;
mod window;
mod window_close;
mod window_create;
mod window_mutate;
mod window_query;
mod workspace;
mod workspace_lifecycle;

use axum::{extract::State, response::Json};

use crate::backend::service::WebReturnType;

use super::AppState;
use crate::backend::service::WebCallType;

// ---- Public surface re-exports (external call sites depend on these) ----

pub(crate) use introspect::{
    agent_layout, agent_tabs, agent_windows, agent_workspaces, resolve_agent_context,
    workspace_id_for_tab, AgentContext,
};
pub(crate) use layout_helpers::setup_torn_off_block_layout;
pub(crate) use object_helpers::{schedule_agent_zoom_mirror, update_object_meta};
pub(crate) use reducer_helpers::{
    dispatch_to_reducer, publish_events, queue_layout_actions_via_reducer, seed_layout_via_reducer,
};

pub(super) async fn handle_service(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Json<WebReturnType> {
    let call: WebCallType = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => return Json(WebReturnType::error(format!("invalid request body: {e}"))),
    };
    Json(run_service_call(&state, &call).await)
}

/// Dispatch a service call and broadcast any resulting `WaveObjUpdate`s to the
/// event bus — the shared core of `handle_service`. Factored out so the typed
/// first-class agent-API verbs (e.g. `/api/v1/window/name`) get byte-identical
/// persistence + broadcast to a raw `/agentmux/service` call without
/// re-implementing it. See SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md.
pub(crate) async fn run_service_call(state: &AppState, call: &WebCallType) -> WebReturnType {
    let service_start = std::time::Instant::now();
    let result = dispatch_service(state, call).await;
    let elapsed = service_start.elapsed();
    // debug, not info: fires on every HTTP /agentmux/service call — a
    // meaningful slice (~10%+) of an unrotated 406 MB launcher-log mirror on
    // a real machine (SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 P1). Default
    // production filter is info, so this is now suppressed unless
    // RUST_LOG=debug is set; still there for perf debugging on demand.
    tracing::debug!(
        "[http-perf] {}.{}: {:.2}ms",
        call.service,
        call.method,
        elapsed.as_secs_f64() * 1000.0,
    );

    // Broadcast every WaveObjUpdate the handler returned so other
    // clients (additional windows, test harnesses, etc.) learn about
    // changes they didn't initiate. The calling HTTP client also gets
    // `updates` in the response body — this broadcast is for
    // everybody else on the event bus. Before this, only a handful
    // of handlers (agent.open, blockcontroller events) broadcast
    // manually, so an external harness's CreateTab / UpdateObject
    // were invisible to the frontend.
    //
    // One batched frame, not one frame per update: these WS frames also
    // reach the CALLING renderer, and they land BEFORE the HTTP response
    // body — so N individual frames repaint the UI in N unbatched steps
    // and the response body's batched application (wos.ts
    // `updateWaveObjects`) arrives too late to matter (version-guarded to
    // a no-op). CloseTab's `[delete tab, update workspace]` pair sent as
    // two frames is exactly the blank-tab flash of
    // SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md §7.
    if let Some(updates) = &result.updates {
        state.event_bus.broadcast_wave_obj_updates(updates);
    }

    result
}

async fn dispatch_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    match call.service.as_str() {
        "object" => object::handle_object_service(state, call).await,
        "client" => client::handle_client_service(state, call).await,
        "credential" => credential::handle_credential_service(state, call).await,
        "window" => window::handle_window_service(state, call).await,
        "workspace" => workspace::handle_workspace_service(state, call).await,
        "host_ipc" => host_ipc::handle_host_ipc_service(state, call).await,
        _ => misc::handle_misc_service(state, call).await,
    }
}
