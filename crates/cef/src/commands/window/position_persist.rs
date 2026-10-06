// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Debounced srv write-through for window position/size — the position-side
// counterpart to `transparency.rs`'s opacity write-through. Closes the gap
// documented in SPEC_PILLAR1_STEP4_CRASH_REPROJECT_2026_07_07.md §4:
// `reproject_from_srv` (the slow-path reproject used when the launcher
// process itself also died, not just the main window closing) had no rect
// to restore secondary windows to, because position/size was only ever
// tracked in the launcher's live in-memory `WindowMirror.last_rect` — gone
// the moment the launcher process exits.
//
// `Window.pos`/`Window.winsize` (crates/srv/src/backend/obj.rs) already
// existed as fields, and the `SetWindowPosAndSize` RPC
// (crates/srv/src/server/service/window_mutate.rs) already existed to
// write them — this file is the first caller of that RPC.

use std::sync::Arc;
use std::time::Duration;

use windows_sys::Win32::Foundation::HWND;

use super::trailing_writer::TrailingWriter;
use crate::state::AppState;

/// Trailing-edge debounce for the srv position write-through, keyed by
/// window label, on one worker thread (`trailing_writer.rs`).
///
/// Position changes arrive far more often than opacity changes (every
/// `EVENT_OBJECT_LOCATIONCHANGE` during a drag, already smoothed to ~20Hz
/// by `position_debounce::should_emit` upstream) and are lower-urgency —
/// nothing reads the persisted value until the next full-restart reproject,
/// unlike opacity which a live crash-recovery path can read moments later.
/// A longer window than opacity's 400ms is deliberate: 1500ms collapses an
/// entire drag-then-release gesture to one write instead of firing on every
/// intermediate pause.
const POSITION_WRITE_DEBOUNCE_MS: u64 = 1500;

static POSITION_WRITER: TrailingWriter =
    TrailingWriter::new("position", Duration::from_millis(POSITION_WRITE_DEBOUNCE_MS));

/// Called from `wrr::win_event`'s `EVENT_OBJECT_LOCATIONCHANGE` handler on
/// every already-20Hz-debounced real (non-pool-move) position report,
/// alongside the existing `launcher_ipc::report_hwnd_position_changed`
/// call. A second, independent consumer of the same position stream — does
/// not touch or replace the launcher-IPC forwarding.
///
/// No-ops (same as opacity's write-through) when the hwnd's label can't be
/// resolved, or when the label has no registered `backend_window_id` yet
/// (e.g. early in window creation, or a floating pane — floating panes
/// have no srv `Window` row, see SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_
/// PERSISTENCE_2026_07_06.md §1.B).
pub(crate) fn report_position_for_srv_writethrough(state: &Arc<AppState>, hwnd: HWND, rect: agentmux_common::ipc::Rect) {
    let Some(label) = state.label_for_hwnd(hwnd) else {
        return;
    };
    let Some(window_id) = state.backend_window_id(&label) else {
        return;
    };

    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    POSITION_WRITER.schedule(label, move || {
        crate::client::backend_set_window_pos_and_size(&web_endpoint, &auth_key, &window_id, rect);
    });
}
