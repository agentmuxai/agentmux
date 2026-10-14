// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The outer-window HWND registry, and what the window under a point is.
//!
//! `HostState.window_hwnds` maps a window label to the HWND of its outer
//! top-level window. It lives here, next to `browsers`, so a relabel moves
//! both in one dispatch and the drop/redock hit tests can ask the window's
//! registered `BrowserKind` instead of parsing its label
//! (SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md R5/R6).
//!
//! The arms emit no events and aren't quit-relevant, so `update` and
//! `host_dispatch` do the same thing for them (`state::window_hwnds` relies
//! on that for its window_pool.rs shim).

use std::collections::HashMap;

use crate::state::BrowserKind;

use super::{DispatchOutput, HostState};

/// Bind `label` to `hwnd`. With `replace: false` an existing binding wins:
/// `capture_hwnd_for_label` must never overwrite the outer HWND the window's
/// creator bound (its own guess can be an inner child or the owner).
pub(super) fn handle_bind_window_hwnd(
    state: &mut HostState,
    label: String,
    hwnd: isize,
    replace: bool,
) -> DispatchOutput {
    if replace || !state.window_hwnds.contains_key(&label) {
        state.window_hwnds.insert(label, hwnd);
    }
    DispatchOutput::default()
}

/// Forget `label`'s HWND. Returns the evicted value; idempotent.
pub(super) fn handle_evict_window_hwnd(state: &mut HostState, label: String) -> DispatchOutput {
    DispatchOutput {
        evicted_window_hwnd: state.window_hwnds.remove(&label),
        ..Default::default()
    }
}

/// The labels bound to `hwnd`, sorted so the answer never depends on map order.
pub(crate) fn labels_at_hwnd(hwnds: &HashMap<String, isize>, hwnd: isize) -> Vec<&str> {
    let mut labels: Vec<&str> = hwnds
        .iter()
        .filter(|(_, &h)| h == hwnd)
        .map(|(l, _)| l.as_str())
        .collect();
    labels.sort_unstable();
    labels
}

/// What a top-level window is to a drop or redock hit test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DropTarget {
    /// A user window that takes the drop: the label its frontend uses.
    Window(String),
    /// A floating pane. Never a target; a Z-order walk looks behind it.
    Floater(String),
    /// No label is bound to this HWND.
    Untracked,
    /// Nothing that takes a drop: a warm pool window, a pane, a popup, a
    /// window whose browser hasn't registered yet, or two labels on one HWND
    /// (Windows reuses a destroyed window's HWND value, so one of them is
    /// stale and nothing says which).
    Nothing,
}

/// [`classify_drop_target`] over this state's HWND registry and browser kinds.
pub(crate) fn drop_target_at(
    state: &HostState,
    hwnd: isize,
    has_backend_window: &dyn Fn(&str) -> bool,
) -> DropTarget {
    classify_drop_target(
        &labels_at_hwnd(&state.window_hwnds, hwnd),
        &|label| state.browsers.get(label).map(|h| h.kind.clone()),
        state.window_hwnds.contains_key("main"),
        has_backend_window,
    )
}

/// Classify the window `labels` are bound to, by the registered kind of its
/// browser (`kind_of`), never by its label's prefix.
///
/// One rename: a user window serving the primary keeps the label it was
/// created under, while its renderer registered as "main". It has no backend
/// window of its own, "main" has one, and "main" has no frame of its own
/// (`main_has_frame`). Its frontend answers to "main", so the drop must name
/// "main" (ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md §0).
pub(crate) fn classify_drop_target(
    labels: &[&str],
    kind_of: &dyn Fn(&str) -> Option<BrowserKind>,
    main_has_frame: bool,
    has_backend_window: &dyn Fn(&str) -> bool,
) -> DropTarget {
    let label = match labels {
        [] => return DropTarget::Untracked,
        [label] => *label,
        _ => return DropTarget::Nothing,
    };
    match kind_of(label) {
        Some(BrowserKind::Floater { .. }) => DropTarget::Floater(label.to_string()),
        Some(kind) if super::quit::is_live_user_window(&kind) => {
            let serves_primary = label != "main"
                && !main_has_frame
                && !has_backend_window(label)
                && has_backend_window("main");
            DropTarget::Window(if serves_primary { "main" } else { label }.to_string())
        }
        _ => DropTarget::Nothing,
    }
}
