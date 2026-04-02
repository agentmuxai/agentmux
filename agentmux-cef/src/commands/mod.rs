// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Command handler modules for the CEF IPC bridge.
// Each module corresponds to a category of commands ported from src-tauri/src/commands/.

pub mod platform;
pub mod window;
pub mod backend;
pub mod providers;
pub mod drag;
pub mod clipboard;
pub mod stubs;

use std::sync::Arc;
use crate::state::AppState;

/// Create an isolated CEF RequestContext for a new browser window.
///
/// Each browser window needs its own renderer process to get an isolated
/// JavaScript context (own `document`, own module state, own SolidJS render tree).
/// CEF assigns a separate renderer process when the RequestContext has a unique
/// `cache_path`. We use `<data_dir>/browser-contexts/<label>/` for this.
pub fn create_isolated_request_context(state: &Arc<AppState>, label: &str) -> Option<cef::RequestContext> {
    let data_dir = state.version_data_dir.lock().unwrap().clone()
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("agentmux-cef-contexts")
                .to_string_lossy()
                .to_string()
        });

    let ctx_path = std::path::PathBuf::from(&data_dir)
        .join("browser-contexts")
        .join(label);
    std::fs::create_dir_all(&ctx_path).ok();

    let settings = cef::RequestContextSettings {
        cache_path: cef::CefString::from(ctx_path.to_str().unwrap_or("")),
        ..Default::default()
    };

    let ctx = cef::request_context_create_context(Some(&settings), None);
    if ctx.is_some() {
        tracing::info!(label = %label, path = %ctx_path.display(), "[cef] created isolated RequestContext");
    } else {
        tracing::warn!(label = %label, "[cef] failed to create isolated RequestContext — falling back to shared");
    }
    ctx
}
