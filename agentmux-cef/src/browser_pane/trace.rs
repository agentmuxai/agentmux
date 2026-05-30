// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Persistent lifecycle trace for browser panes.
//!
//! **DO NOT REMOVE.** Browser-pane re-create under tear-off/redock churn
//! (create-while-Closing — #1168 — and the black-render-on-redock that
//! followed) is a recurring race surface. This trace is kept in
//! permanently so the *next* investigation starts from data, not guesswork.
//!
//! Every event carries a process-global monotonic `seq` and the pane's
//! `block` id, so the full interleaved churn (several panes
//! creating/closing/loading at once) can be reconstructed from one log.
//!
//! Filter: `muxlog host pane-trace`.

use std::sync::atomic::{AtomicU64, Ordering};

static PANE_TRACE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Emit one browser-pane lifecycle trace line.
///
/// - `block_id` — the pane's block id (logged truncated to 8 chars).
/// - `event` — lifecycle phase, one of: `create-request` (a create was
///   asked for), `create-deferred-closing` (create deferred because the
///   prior pane is still Closing; the reducer replays it on close-completion),
///   `create-parent` (the resolved parent window + HWND, Windows),
///   `load-end`, `load-error`, `close`.
/// - `detail` — free-form context (url, error code, label, …).
pub fn pane_trace(block_id: &str, event: &str, detail: &str) {
    let seq = PANE_TRACE_SEQ.fetch_add(1, Ordering::Relaxed);
    let block: String = block_id.chars().take(8).collect();
    tracing::info!(
        target: "pane-trace",
        "[pane-trace] seq={} block={} event={} {}",
        seq,
        block,
        event,
        detail,
    );
}
