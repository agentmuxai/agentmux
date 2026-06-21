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
pub mod tear_off_hook;
pub mod window_pool;
pub mod clipboard;
pub mod stubs;
pub mod palette;
pub mod orphan_reconcile;
pub mod floating_pane;

use std::sync::Arc;
use crate::state::AppState;

/// Create an isolated CEF RequestContext for a new browser window.
///
/// Each browser window needs its own renderer process to get an isolated
/// JavaScript context (own `document`, own module state, own SolidJS render tree).
/// CEF assigns a separate renderer process when the RequestContext has a unique
/// `cache_path`. We use `<root_cache_path>/browser-contexts/<label>/` — it MUST
/// be a child of `Settings.root_cache_path` (the cef-cache dir) or CEF rejects it
/// and falls back to in-memory storage. SPEC_CEF_LOG_ROBUSTNESS_2026_06_20.md §1.
pub fn create_isolated_request_context(state: &Arc<AppState>, label: &str) -> Option<cef::RequestContext> {
    // Phase 1 diagnostic tracing (added 2026-05-02 freeze investigation, see
    // docs/specs/SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md). The freeze
    // wedges the UI thread inside CEF's Chrome profile-init under concurrent
    // load; we need to find the EXACT line that silences before committing
    // to the runner-based serialization fix.
    let t0 = std::time::Instant::now();
    tracing::info!(label = %label, "[cef-profile-init] entering create_isolated_request_context");

    // Root the per-window context UNDER root_cache_path (the cef-cache dir) so
    // CEF accepts it — it requires cache_path to be a descendant of
    // root_cache_path, else it rejects it and falls back to in-memory storage
    // (SPEC_CEF_LOG_ROBUSTNESS §1). Fall back to a temp dir only if
    // root_cache_path isn't initialized yet (pre-boot edge).
    let cache_root = state.cef_cache_dir.lock().clone()
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("agentmux-cef-contexts")
                .to_string_lossy()
                .to_string()
        });

    let ctx_path = std::path::PathBuf::from(&cache_root)
        .join("browser-contexts")
        .join(label);
    // Do NOT pre-create the directory — CEF's Chrome profile initializer
    // (chrome_browser_context.cc) fails when the directory already exists but
    // has no valid profile structure. Let CEF create and initialize it.

    let settings = cef::RequestContextSettings {
        cache_path: cef::CefString::from(ctx_path.to_str().unwrap_or("")),
        persist_session_cookies: 0,
        ..Default::default()
    };

    tracing::info!(
        label = %label,
        elapsed_us = t0.elapsed().as_micros() as u64,
        "[cef-profile-init] calling request_context_create_context"
    );
    let ctx = cef::request_context_create_context(Some(&settings), None);
    tracing::info!(
        label = %label,
        elapsed_us = t0.elapsed().as_micros() as u64,
        ok = ctx.is_some(),
        "[cef-profile-init] request_context_create_context returned"
    );

    if ctx.is_some() {
        tracing::info!(label = %label, path = %ctx_path.display(), "[cef] created isolated RequestContext");
    } else {
        tracing::warn!(label = %label, "[cef] failed to create isolated RequestContext — falling back to shared");
    }
    ctx
}
