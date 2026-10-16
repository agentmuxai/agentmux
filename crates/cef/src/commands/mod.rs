// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Command handler modules for the CEF IPC bridge.
// Each module corresponds to a category of commands ported from src-tauri/src/commands/.

pub mod platform;
pub mod autostart;
pub mod taskbar_attention;
pub mod cli_login;
mod login_console;
pub mod window;
pub mod backend;
pub mod providers;
pub mod drag;
pub mod tear_off_hook;
pub mod window_pool;
pub mod replace_page;
pub mod pane_pool;
pub mod startup_pools;
pub mod clipboard;
pub mod stubs;
pub mod palette;
pub mod orphan_reconcile;
pub mod floating_pane;
/// What went wrong and what AgentMux did about it, kept for the user.
pub mod incidents;



/// The RequestContext a secondary top-level AgentMux window is created with
/// (a cold "Open in New Window", a tear-off target, a warm pool window):
/// `None`, the shared default context — the one the main window, floating
/// panes and browser panes already use.
///
/// Until 2026-10-13 each of these windows got its own unique off-the-record
/// profile (an empty `cache_path`), on the belief that a window needs its own
/// profile to get its own renderer process and JavaScript context. It
/// doesn't: every top-level browser has its own document, globals and module
/// state whichever process it lands in, and Chromium gives a new top-level
/// browser of the same site its own process anyway (measured: a floating pane
/// on the shared context runs in a renderer of its own). What the private
/// profile did cost was its in-memory-only cache: a cold new window
/// re-downloaded and recompiled the whole app (7 of 7 JS files, 4.4 MB, none
/// from cache), where a floating pane on the shared context loads all of them
/// from the disk cache and V8's code cache. It also ran Chrome's profile
/// initialization on the UI thread for every window (the subject of the
/// 2026-05-02 freeze investigation). Same-origin storage (`localStorage`,
/// cookies) is shared with the main window, exactly as it already is for
/// floating panes; `sessionStorage` stays per window.
///
/// Real per-window disk profiles (`<root>/ctx-<label>`) remain unsafe here —
/// browser creation stalls (verified 2026-07-09) — and are not used.
/// docs/specs/SPEC_WINDOW_OPEN_LATENCY_AND_WARM_POOLS_2026_10_13.md §2.4.
pub fn secondary_window_request_context() -> Option<cef::RequestContext> {
    None
}

/// Delete on-disk litter left by earlier isolated-context path schemes.
///
/// Per-window contexts were in-memory (empty cache_path) from the 2026-07-09
/// fix until 2026-10-13 (secondary windows now share the default context), so
/// nothing under cef-cache belongs to them anymore. Two legacy
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
