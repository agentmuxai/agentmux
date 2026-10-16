// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Whether a CEF Views window is maximized, read on the UI thread for the
// `is_window_maximized` IPC on macOS / Linux (Windows answers from the reducer
// or IsZoomed directly, so this module is not built there). SPEC_FLOATING_PANE_WINDOW_CONTROLS_REFINEMENTS_2026_10_16 §4.2.

use std::sync::Arc;
use cef::*;
use crate::state::AppState;
use super::get_window_on_ui;

wrap_task! {
    pub struct GetWindowMaximizedTask {
        state: Arc<AppState>,
        label: String,
        tx: std::sync::mpsc::SyncSender<Option<bool>>,
    }

    impl Task {
        fn execute(&self) {
            let maximized = get_window_on_ui(&self.state, &self.label).map(|w| w.is_maximized() != 0);
            let _ = self.tx.try_send(maximized);
        }
    }
}

/// Whether a CEF Views window is maximized, read from the IPC thread by
/// bouncing through the UI thread. `None` if the window isn't found or the UI
/// thread doesn't answer within the timeout.
pub fn get_window_maximized_blocking(state: &Arc<AppState>, label: &str) -> Option<bool> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<bool>>(1);
    let mut task = GetWindowMaximizedTask::new(state.clone(), label.to_string(), tx);
    post_task(ThreadId::UI, Some(&mut task));
    rx.recv_timeout(std::time::Duration::from_millis(250)).ok().flatten()
}
