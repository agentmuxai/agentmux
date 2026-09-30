// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/// Window role in the AgentMux multi-window model.
///
/// Two distinct types with different taskbar treatment:
/// - `FullInstance`: independent AgentMux window (like Chrome/VS Code new window).
///   Appears in the Windows taskbar. All user-facing "new window" paths (status-bar
///   version click, second `agentmux.exe` launch, `Ctrl+Shift+N`) create one.
/// - `Subwindow`: hidden from the taskbar via `ITaskbarList::DeleteTab`. Only
///   reachable through the backend `open_subwindow` API — reserved for agent /
///   internal use cases (transient auxiliary views, tool-spawned panels). Closes
///   when its parent full instance closes.
///
/// The type itself lives in `agentmux_common::ipc` — it is the wire type the
/// launcher deserializes, and the host used to carry a byte-identical private
/// copy of it (same variants, same derives, same `serde(rename_all)`) that
/// `client/lifecycle.rs` then mapped back onto the common one variant by
/// variant. One definition; the mapping is gone
/// (`docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md` §2.2).
pub use agentmux_common::ipc::WindowKind;

/// Per-window metadata held alongside the CEF `Browser`. See `WindowKind` for
/// the semantics of `kind` and `parent_instance_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowMeta {
    pub label: String,
    pub kind: WindowKind,
    /// For `Subwindow` only: label of the `FullInstance` that owns this window.
    /// `None` for `FullInstance`.
    pub parent_instance_id: Option<String>,
}

/// Phase B.5 (window_meta step d) — pre-create handoff. Caller
/// (`drag.rs::tear_off`, `commands/window.rs::open_new_window`,
/// `window_pool.rs::spawn_pool_window`, `pane/creation.rs`) pushes
/// one entry per window CEF is about to create; `client.rs::on_after_created`
/// pops the head entry and uses `kind` for the Subwindow
/// taskbar-hide branch + as the payload for `ReportWindowOpened`.
///
/// Replaces the previous `pending_window_labels: VecDeque<String>`
/// queue + parallel caller-side `window_meta` writes that used to
/// act as the kind/parent channel. Collapsing them into a single
/// tuple eliminates the parallel-write race; on_after_created
/// performs the single canonical `window_meta.insert` from the
/// popped entry (kept as a synchronous host-side cache for
/// open_subwindow's parent liveness check + cascade-close
/// enumeration in `task dev` mode where launcher IPC is absent).
#[derive(Clone, Debug)]
pub struct PendingWindowCreation {
    pub label: String,
    pub kind: WindowKind,
    pub parent_instance_id: Option<String>,
}
