// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

pub(crate) mod cli_handlers;
mod files;
pub(crate) mod app_api;
// `pub` so `bootstrap::install_agent_turn_delivery` can reach `run_agent_turn`
// to start a turn outside the RPC path.
pub mod agent_resources;
pub mod agent_handlers;
mod agent_takeover;
mod editor_handlers;
mod fs_handlers;
pub(crate) mod identity_auth_dirs;
mod identity_auth_persist;
mod identity_auth_spawn;
mod identity_handlers;
pub mod install_handlers;
mod lsp_handlers;
mod system_install_handlers;
mod messagebus;
// pub(crate): muxbus::cloud_subscriber (the WAN delivery path) calls
// resolve_transcript_request_tier_fields directly — see that function's
// own doc comment for why.
pub(crate) mod reactive;
mod name_resolution;
pub(crate) mod caller;
pub(crate) mod agent_self_keys;
pub(crate) mod jekt_held;
pub(crate) mod actor;
pub(crate) mod service;
mod shell_handlers;
mod tool_handlers;
mod providers_handlers;
mod voice;
mod attachments;
pub(crate) mod mux_obj_bridge;
mod websocket;
mod drone_handlers;
mod cron;
pub(crate) mod work_queue;
mod messaging_handlers;
mod muxbus_handlers;
mod muxspect_handlers;
pub(crate) mod native_memory_handlers;
pub(crate) mod memory_delivery_handlers;
mod notify_handlers;
pub(crate) mod browser_allowlist;
pub(crate) mod browser_attention;
pub(crate) mod browser_host_sync;
pub(crate) mod browser_identity;
pub(crate) mod browser_owner;
pub(crate) mod browser_popup;
pub(crate) mod browser_uploads;
pub(crate) mod ui_handlers;
pub(crate) mod ui_shortcuts;
pub(crate) mod widget_access_handlers;
pub(crate) mod widget_agent_handlers;
pub(crate) mod widget_handlers;

#[cfg(test)]
pub(crate) mod tests;

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Extension, Query, Request, State},
    http::{header, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, patch, post},
    Router,
};
use agentmux_common::secret_eq::secret_eq;
use serde_json::json;
use tower_http::cors::{Any, CorsLayer};

use crate::backend::eventbus::EventBus;
use crate::backend::lan_discovery::LanDiscoveryController;
use crate::backend::lsp::LspSupervisor;
use crate::backend::messagebus::MessageBus;
use crate::backend::reactive::{Poller, ReactiveHandler};
use crate::backend::storage::filestore::FileStore;
use crate::backend::blockcontroller;
use crate::backend::storage::store::Store;
use crate::backend::history::HistoryService;
use crate::backend::subagent_watcher::SubagentWatcher;
use crate::backend::wconfig;
use crate::backend::mps::Broker;
use agentmux_common::api_types::{
    PaneTitleRequest, PtyShellCreateRequest, PtyShellCreateResponse, PtyShellInputRequest,
    PtyShellInputResponse, PtyShellReadRequest, PtyShellReadResponse, PtyShellResizeRequest,
    PtyShellResizeResponse, PtyShellStatusRequest,
    PtyShellStatusResponse, PtyShellStopRequest, PtyShellStopResponse, ShellCreateRequest,
    ShellCreateResponse, ShellInputFailure, ShellInputRequest, ShellInputResponse,
    ShellStatusRequest, ShellStatusResponse, ShellStopRequest, TabActivateRequest, TabNameRequest,
    TabNewRequest, WindowFocusRequest, WindowNameRequest, WorkspaceNameRequest, WpsPublishRequest,
};

// ---- AppState ----

mod state;
mod routes;
mod http_health;
mod http_fleet;
mod http_viewer;
mod http_shell;
mod http_pty_shell;
mod http_open;
mod http_app_api;
mod http_self;
mod http_naming;
mod http_layout;
mod auth;
pub use state::*;
pub use routes::*;
pub use http_viewer::build_viewer_router;
use http_health::*;
use http_fleet::*;
pub(crate) use http_shell::*;
use http_pty_shell::*;
use http_open::*;
use http_app_api::*;
use http_self::*;
use http_naming::*;
use http_layout::*;
pub(crate) use auth::*;
