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
/// CEF assigns a separate renderer process when the RequestContext maps to a
/// unique profile. We request that with an EMPTY `cache_path`: Chrome's profile
/// initializer (chrome_browser_context.cc) then creates a new/unique
/// OffTheRecord profile — isolation without any on-disk profile. That is
/// deliberate, not a compromise:
///
/// - Labels embed a per-run UUID, so a disk-backed profile could never be
///   reused across runs — persistence has no value here. (Browser panes,
///   where cookie/storage persistence DOES matter, use the shared default
///   context — `None` — which is disk-backed.)
/// - A non-empty `cache_path` must be a DIRECT child of
///   `Settings.root_cache_path` (`cache_path.DirName() == user_data_dir`);
///   anything deeper logs `Cannot create profile at path …` and falls back to
///   the same unique-OTR profile. The old `<root>/browser-contexts/<label>/`
///   layout was one level too deep and hit that error on every isolated
///   window since v0.33.x (2–22 log lines per launch).
/// - A VALID direct-child path (`<root>/ctx-<label>`) creates a real Chrome
///   profile — but browser creation then stalls indefinitely in this host
///   (no on_after_created / page load; verified live 2026-07-09 on a dev
///   build). Real per-window profiles are NOT safe in AgentMux's
///   alloy-style-views-on-chrome-bootstrap setup.
///
/// So: empty path = the same unique-OTR behavior the error-fallback has been
/// (accidentally) providing all along, minus the error line and the risk.
/// SPEC_CEF_LOG_ROBUSTNESS_2026_06_20.md §1.6.
pub fn create_isolated_request_context(_state: &Arc<AppState>, label: &str) -> Option<cef::RequestContext> {
    // Phase 1 diagnostic tracing (added 2026-05-02 freeze investigation, see
    // docs/specs/SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md). The freeze
    // wedges the UI thread inside CEF's Chrome profile-init under concurrent
    // load; we need to find the EXACT line that silences before committing
    // to the runner-based serialization fix.
    let t0 = std::time::Instant::now();
    tracing::info!(label = %label, "[cef-profile-init] entering create_isolated_request_context");

    // Empty cache_path → chrome_browser_context.cc skips the disk-profile
    // branches entirely and creates a new/unique OffTheRecord profile, with
    // no error logged and nothing written to disk. See doc comment above for
    // why the disk-backed alternatives are wrong (grandchild path → error +
    // this same fallback; direct-child path → browser creation stalls).
    let settings = cef::RequestContextSettings {
        cache_path: cef::CefString::from(""),
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
        tracing::info!(label = %label, "[cef] created isolated RequestContext (unique off-the-record profile)");
    } else {
        tracing::warn!(label = %label, "[cef] failed to create isolated RequestContext — falling back to shared");
    }
    ctx
}

/// Delete on-disk litter left by earlier isolated-context path schemes.
///
/// Isolated contexts are in-memory (empty cache_path) as of the 2026-07-09
/// fix, so nothing under cef-cache belongs to them anymore. Two legacy
/// layouts left dirs behind:
/// - `browser-contexts/<label>/` — the grandchild layout (≤0.51.x). The
///   profile itself never initialized (fell back to OTR), but CEF's
///   request-context layer still dropped partial cache dirs there.
/// - `ctx-<label>/` — the short-lived direct-child experiment (real Chrome
///   profiles; never released — the layout stalled browser creation).
///
/// Labels embed a per-run UUID, so none of these dirs can ever be referenced
/// again; delete on sight. Runs off-thread — the trees can hold many small
/// files and this sits on the startup path.
pub fn cleanup_legacy_context_dirs(cache_root: &str) {
    let root = std::path::PathBuf::from(cache_root);
    let mut stale: Vec<std::path::PathBuf> = Vec::new();

    let legacy = root.join("browser-contexts");
    if legacy.is_dir() {
        stale.push(legacy);
    }
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("ctx-")
                && entry.path().is_dir()
            {
                stale.push(entry.path());
            }
        }
    }
    if stale.is_empty() {
        return;
    }
    tracing::info!(
        count = stale.len(),
        "[cef] removing legacy isolated-context dirs"
    );
    std::thread::spawn(move || {
        for dir in stale {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                tracing::warn!(
                    dir = %dir.display(),
                    error = %e,
                    "[cef] failed to remove legacy isolated-context dir"
                );
            }
        }
    });
}
