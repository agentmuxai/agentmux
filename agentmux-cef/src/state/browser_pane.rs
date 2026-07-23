// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ── Phase H — host reducer buildout ──────────────────────────────────────
//
// All types below are reducer-only state. PR #1 (h1-foundations) declares
// them; subsequent PRs (#2-#5) wire callers through the reducer per the
// a→b→c→d→e migration ratchet. See:
//   docs/specs/SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md
//   docs/specs/SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md
//
// These types intentionally have `#[allow(dead_code)]` because PR #1 ships
// the scaffolding without callers — fields are populated by reducer arms but
// no production code reads them yet. Subsequent PRs lift the allow as they
// wire each migration.

// ── Pane lifecycle (H.1) ─────────────────────────────────────────────────

/// Lifecycle state of a browser pane (the `defwidget@browser` widget). Held
/// inside `HostState.browser_panes` keyed by `block_id`. Mirrors the existing
/// `PaneStateMachine::BrowserPaneLifecycle` (pane/lifecycle.rs:28); the existing
/// type stays during PR #2's a→e migration. PR #2 step e deletes the
/// pane/lifecycle.rs version and migrates all readers to this one.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BrowserPaneLifecycle {
    /// Pane is alive and accepting operations (focus, resize, navigate).
    Live,
    /// Close requested; awaiting CEF on_before_close to fully tear down.
    /// `since` carries the request timestamp for diagnostic purposes only;
    /// nothing in the reducer is timer-driven.
    Closing { since: std::time::Instant },
}

/// Per-pane reducer-managed entry. Replaces `pane::lifecycle::BrowserPaneEntry`
/// (lifecycle.rs:42) at PR #2 step e.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct BrowserPaneEntry {
    pub block_id: String,
    pub label: String,
    pub lifecycle: BrowserPaneLifecycle,
    /// The window this pane was created in (`main`, `floating-<uuid>`, …).
    /// Used to detect a cross-window move (tear-off / redock): a create
    /// request whose `window_label` differs from this must NOT be served by
    /// re-navigating the existing browser in the OLD window (that leaves the
    /// requested window black). See the `AlreadyLiveElsewhere` handling in
    /// `reducer/panes.rs` + `browser_panes.rs`.
    pub window_label: String,
}

// ── Pane window-placement state (pane-state reducer, Phase 0) ─────────────
//
// SPEC_PANE_STATE_REDUCER_2026-05-28.md (REVISION 2026-05-29 — folded into
// HostState rather than a standalone PaneStateMachine, mirroring the Phase-H
// consolidation that deleted `pane::lifecycle::PaneStateMachine`).
//
// This tracks the OS-window placement of a FLOATING pane (its
// maximize/restore state + the rect to restore to). It is deliberately
// SEPARATE from `BrowserPaneEntry`/`BrowserPaneLifecycle` above, which own
// Live/Closing lifecycle: lifecycle stays in `HostState.browser_panes`;
// placement lives in `HostState.pane_window_states`. Docked panes have NO
// entry here — their "maximize" is backend magnify
// (`LayoutState.magnifiednodeid`), routed by the frontend `<MaximizeButton>`
// (spec §3.3a, b2), never through this reducer.

/// Screen-space window rectangle in physical pixels. Distinct from
/// `browser_api::Rect` (web/CSS coordinates) — this is for native window
/// placement (the floating pane's outer HWND rect).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub struct PaneRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// OS-window placement for a floating pane. The shared maximize button's
/// floating half toggles between `Normal` and `Maximized`; `Minimized` is
/// OS-reported (Win+Down / system) and orthogonal — a minimized floater
/// un-minimizes back to whichever of Normal/Maximized it held before, so
/// the reducer remembers the pre-minimize placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum WindowPlacement {
    #[default]
    Normal,
    Maximized,
    Minimized,
}

/// Per-floater window-placement entry, keyed by the floating-window LABEL
/// (`floating-<uuid>`) in `HostState.pane_window_states`. Holds ONLY the
/// floating window's OS placement and the rect to restore to after
/// un-maximize. Keyed by label (not block_id) because floaters are tracked
/// by window label everywhere (`window_hwnds`, the `?windowLabel=` URL, the
/// `on_before_close` teardown) and are not in `browser_panes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct PaneWindowState {
    pub placement: WindowPlacement,
    /// Rect to restore to after un-maximize / un-minimize. `None` until a
    /// normal-mode rect has been observed (replaces the deleted
    /// `AppState.floating_restored_rects` stash — spec §4).
    pub last_known_normal_rect: Option<PaneRect>,
}
