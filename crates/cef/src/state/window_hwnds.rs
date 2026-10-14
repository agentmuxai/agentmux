// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `AppState` access to the outer-window HWND registry, which the host
//! reducer owns (`HostState.window_hwnds`, `reducer/window_hwnds.rs`).

use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};

use crate::reducer::{DropTarget, HostCommand, HostState};

use super::AppState;

/// `AppState::window_hwnds`: a handle onto `HostState.window_hwnds`.
pub struct WindowHwndsShim(Arc<Mutex<HostState>>);

impl WindowHwndsShim {
    pub(super) fn new(host_state: Arc<Mutex<HostState>>) -> Self {
        Self(host_state)
    }

    /// Only for the `state.window_hwnds.lock()` call sites left in
    /// `commands/window_pool.rs`; everything else uses the `AppState` methods
    /// below. Holds the `host_state` lock for as long as the guard lives, so
    /// keep it to one statement.
    pub fn lock(&self) -> WindowHwndsGuard<'_> {
        WindowHwndsGuard(self.0.lock())
    }
}

/// The map's old `get` / `insert` / `remove`, with writes going through the
/// reducer's arms. Calling `update` directly matches `host_dispatch` here:
/// those arms emit no events and aren't quit-relevant.
pub struct WindowHwndsGuard<'a>(MutexGuard<'a, HostState>);

impl WindowHwndsGuard<'_> {
    pub fn get(&self, label: &str) -> Option<&isize> {
        self.0.window_hwnds.get(label)
    }

    pub fn insert(&mut self, label: String, hwnd: isize) {
        crate::reducer::update(&mut self.0, HostCommand::BindWindowHwnd { label, hwnd, replace: true });
    }

    pub fn remove(&mut self, label: &str) -> Option<isize> {
        crate::reducer::update(&mut self.0, HostCommand::EvictWindowHwnd { label: label.to_string() })
            .evicted_window_hwnd
    }
}

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
