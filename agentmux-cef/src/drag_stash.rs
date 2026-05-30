// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! In-process stash for file paths captured by `CefDragHandler::on_drag_enter`
//! and consumed by the `consume_drag_paths` IPC at JS drop time.
//!
//! Why: the HTML5 drop event in a CEF browser only surfaces bare filenames,
//! not full filesystem paths. CEF's DragData exposes the real paths, but only
//! during the OnDragEnter callback. We stash them here for the JS-side drop
//! handler to consume a few milliseconds later.
//!
//! Single-slot design: only one drag is active in the OS at a time. A new
//! OnDragEnter overwrites the previous stash. A 5-second TTL evicts stale
//! entries left behind when a drag enters but the user drops back into the
//! source app instead of an AgentMux pane.
//!
//! Spec: docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md §3.3.

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(5);

static STASH: LazyLock<Mutex<Option<(Instant, Vec<String>)>>> =
    LazyLock::new(|| Mutex::new(None));

pub fn put(paths: Vec<String>) {
    *STASH.lock().unwrap() = Some((Instant::now(), paths));
}

/// Take the stash if non-empty and within TTL. Returns the paths and clears
/// the slot. If TTL expired, the slot is cleared and an empty vec is returned.
pub fn take() -> Vec<String> {
    let mut g = STASH.lock().unwrap();
    if let Some((ts, _)) = g.as_ref() {
        if ts.elapsed() > TTL {
            *g = None;
            return Vec::new();
        }
    }
    g.take().map(|(_, p)| p).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_take_returns_paths_and_clears() {
        put(vec!["/tmp/a".to_string(), "/tmp/b".to_string()]);
        let out = take();
        assert_eq!(out, vec!["/tmp/a".to_string(), "/tmp/b".to_string()]);
        assert!(take().is_empty());
    }

    #[test]
    fn empty_take_when_nothing_stashed() {
        // Ensure clean state in case another test ran first.
        let _ = take();
        assert!(take().is_empty());
    }
}
