// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use cef::Browser;

// ── Browser handle registry (H.2) ────────────────────────────────────────

/// Wrapped CEF Browser handle stored in `HostState.browsers`. Replaces the
/// raw `Mutex<HashMap<String, Browser>>` at `state.rs::AppState.browsers`
/// at PR #2 step e.
///
/// `cef::Browser` is `Clone` (refcounted FFI handle) and safe to store
/// inside the reducer's mutex-guarded state. Doesn't impl Debug, hence
/// the manual `impl Debug` below for `BrowserHandle`.
#[derive(Clone)]
#[allow(dead_code)]
pub struct BrowserHandle {
    pub label: String,
    pub browser: Browser,
    pub kind: BrowserKind,
    pub registered_at: std::time::Instant,
}

impl std::fmt::Debug for BrowserHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserHandle")
            .field("label", &self.label)
            .field("kind", &self.kind)
            .field("registered_at", &self.registered_at)
            .field("browser", &"<cef::Browser>")
            .finish()
    }
}

/// Distinguishes top-level CEF Browsers (full-instance windows + pool
/// windows) from pane CEF Browsers (children of a top-level). Determines
/// taskbar treatment, lifecycle ownership, etc.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum BrowserKind {
    /// Top-level window (`main`, `window-*`, and the window pool). `is_pool=true`
    /// while the window is in the warm window pool; cleared on promote. This is
    /// the ONLY kind that keeps the instance alive — see `is_live_user_window`.
    TopLevel { is_pool: bool },
    /// Floating pane / tear-off in its own frameless `WS_POPUP` window —
    /// `floating-<uuid>` (created directly) or `floating-pool-<uuid>` (taken from
    /// the pane pool). `is_pool=true` while warm in the pane pool; cleared on
    /// promote (`pane_pool.rs`). Floaters do **not** keep the instance alive
    /// (invariant FP-LIFE) — `is_live_user_window` counts only
    /// `TopLevel { is_pool: false }`, so a Floater is excluded by type, not by a
    /// label-prefix string. They ARE trusted top-level renderers, so
    /// `list_top_level_browsers` includes them for host JS-event emission.
    Floater { is_pool: bool },
    /// Browser pane child window. `block_id` correlates with the
    /// `HostState.browser_panes` entry.
    Pane { block_id: String },
    /// A transient OAuth/OIDC sign-in popup (`window.open` from a browser pane
    /// to an authorization endpoint — see `on_before_popup`). CEF owns its
    /// window; we track it only to close that window when the auth browser
    /// tears down. Does NOT keep the instance alive (`is_live_user_window`
    /// counts only `TopLevel { is_pool: false }`), so it's excluded from the
    /// last-window quit gate by type — the register/unregister of a popup
    /// during its close never perturbs the watchdog. Skips the full-window
    /// treatment (focus-restore / OS-close-routing / floater-cascade hooks,
    /// launcher FullInstance registration) a real top-level window gets.
    Popup,
}
