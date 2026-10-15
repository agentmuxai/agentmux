// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `AppState` access to the outer-window HWND registry (label → outer
//! top-level HWND), which the host reducer owns (`HostState.window_hwnds`,
//! `reducer/window_hwnds.rs`). Bound at Views window creation (`app/mod.rs`
//! `on_window_created`) and by the floating-pane creator right after
//! `CreateWindowExW`; `capture_hwnd_for_label` fills in any label still
//! missing once the window is shown. Every write goes through the reducer.

use std::collections::HashSet;

use crate::reducer::{DropTarget, HostCommand};

use super::AppState;

impl AppState {
    /// The outer top-level HWND bound to `label`, as its raw value.
    pub fn window_hwnd(&self, label: &str) -> Option<isize> {
        self.host_state.lock().window_hwnds.get(label).copied()
    }

    /// Bind `label` to its outer top-level `hwnd`, replacing any earlier one.
    pub fn bind_window_hwnd(&self, label: &str, hwnd: isize) {
        self.host_dispatch(HostCommand::BindWindowHwnd { label: label.to_string(), hwnd, replace: true });
    }

    /// Bind `label` to `hwnd` unless it already has one (checked under the
    /// same lock as the write).
    pub fn bind_window_hwnd_if_absent(&self, label: &str, hwnd: isize) {
        self.host_dispatch(HostCommand::BindWindowHwnd { label: label.to_string(), hwnd, replace: false });
    }

    /// Forget `label`'s HWND; returns what it was bound to.
    pub fn evict_window_hwnd(&self, label: &str) -> Option<isize> {
        self.host_dispatch(HostCommand::EvictWindowHwnd { label: label.to_string() })
            .evicted_window_hwnd
    }

    /// Every HWND some label is bound to.
    pub fn bound_window_hwnds(&self) -> HashSet<isize> {
        self.host_state.lock().window_hwnds.values().copied().collect()
    }

    /// The label bound to `hwnd` (the first in sort order if a stale entry
    /// shares it). Maps an OS-level hide or move back to a window, covering
    /// close paths (Alt+F4, taskbar) that bypass the `close_window()` RPC.
    pub fn label_for_hwnd(&self, hwnd: windows_sys::Win32::Foundation::HWND) -> Option<String> {
        let st = self.host_state.lock();
        crate::reducer::labels_at_hwnd(&st.window_hwnds, hwnd as isize)
            .first()
            .map(|l| l.to_string())
    }

    /// What the top-level window `hwnd` is to a drop or redock hit test, by
    /// its registered `BrowserKind` (`reducer::drop_target_at`).
    pub fn drop_target_at(&self, hwnd: isize) -> DropTarget {
        // Snapshot first: never hold another lock while `host_state` is held.
        let with_backend: HashSet<String> = self.shadow_backend_window_ids.lock().keys().cloned().collect();
        crate::reducer::drop_target_at(&self.host_state.lock(), hwnd, &|label| with_backend.contains(label))
    }

    /// Labels of the live user windows (`BrowserKind::TopLevel { is_pool: false }`).
    pub fn user_window_labels(&self) -> HashSet<String> {
        crate::reducer::live_user_window_labels(&self.host_state.lock()).into_iter().collect()
    }
}
