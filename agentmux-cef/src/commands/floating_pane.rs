// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Phase 1 of the floating-pane tear-off feature (issue #810 + spec
//! `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md`).
//!
//! This module hosts the IPC command and Win32-native primitive that
//! creates a *subordinate* floating window — a free-positioned palette-
//! style window OWNED by the source AgentMux main window. Unlike the
//! existing tab tear-off path (which spawns a full new AgentMux
//! instance), a floating pane:
//!
//! - has no taskbar entry (`WS_EX_TOOLWINDOW`),
//! - has no Alt-Tab entry (also `WS_EX_TOOLWINDOW`),
//! - minimizes / restores / destroys with its owner,
//! - shares the source instance's sidecar, data dir, and reducer state.
//!
//! Phase 1 ships **only** the windowing primitive. The browser embedded
//! in the floating window loads
//! `<frontend>?floatingPaneId=<id>&windowLabel=floating-<n>` and the
//! frontend renders a minimal placeholder shell that says "Floating
//! pane: \<id\>". Wiring the *drag-out gesture* to this primitive is
//! Phase 3. Wiring the full `<Block>` renderer is Phase 2.
//!
//! Non-Windows platforms are out of scope per spec §10. The macOS path
//! is `NSWindow::addChildWindow`; Linux varies by compositor. Both will
//! be addressed in a follow-up.

use std::sync::Arc;

use serde::Deserialize;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct OpenFloatingPaneArgs {
    /// Reducer-side identifier for the pane being torn off. Threaded
    /// through to the frontend via the query string so the floating-
    /// pane shell knows what to render.
    pub pane_id: String,
    /// Screen-space top-left coordinates where the new floating window
    /// should appear. Typically the cursor position at drop time.
    pub x: i32,
    pub y: i32,
    /// Initial window size. Phase 6 will memo the source pane's last
    /// docked size; Phase 1 accepts a caller-provided default.
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, serde::Serialize)]
pub struct OpenFloatingPaneResponse {
    /// The window label assigned to the floating window. Stable for
    /// the life of the floater; persists into `state.window_meta` like
    /// any other top-level label.
    pub window_label: String,
}

/// IPC handler — called when the frontend or an agent invokes
/// `open_floating_pane_window` on the host. Validates input, allocates
/// a stable label, and posts a UI-thread task to create the owned HWND
/// and embed a CEF browser inside it.
pub fn open_floating_pane_window(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let parsed: OpenFloatingPaneArgs = serde_json::from_value(args.clone())
        .map_err(|e| format!("open_floating_pane_window: invalid args: {e}"))?;

    if parsed.pane_id.is_empty() {
        return Err("open_floating_pane_window: pane_id is required".to_string());
    }
    if parsed.width <= 0 || parsed.height <= 0 {
        return Err(format!(
            "open_floating_pane_window: width/height must be positive (got {}×{})",
            parsed.width, parsed.height
        ));
    }

    // The H.7 main-window-creation gate (any pane mid-close → wedged
    // Chromium IPC) applies here too — same Chromium message loop. If
    // a pane is closing, refuse the floating-window creation; the
    // caller retries.
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_floating_pane_window refused — pane is mid-close (H.7 invariant)"
        );
        return Err("a pane is currently closing; retry shortly".to_string());
    }

    let window_id = uuid::Uuid::new_v4();
    let window_label = format!("floating-{}", window_id.simple());

    tracing::info!(
        pane_id = %parsed.pane_id,
        label = %window_label,
        x = parsed.x,
        y = parsed.y,
        w = parsed.width,
        h = parsed.height,
        "[floating-pane] open_floating_pane_window request",
    );

    #[cfg(target_os = "windows")]
    {
        crate::floating_pane::post_create_floating_window(state, &parsed, &window_label);
        Ok(serde_json::to_value(OpenFloatingPaneResponse { window_label }).unwrap_or_default())
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Phase 1 is Windows-only per spec §10. macOS uses
        // `NSWindow::addChildWindow`; Linux varies. Both are explicit
        // follow-ups; return a clear error so callers fail loudly.
        let _ = parsed;
        let _ = window_label;
        Err(
            "open_floating_pane_window: not yet implemented on this platform (Windows only in Phase 1)"
                .to_string(),
        )
    }
}
