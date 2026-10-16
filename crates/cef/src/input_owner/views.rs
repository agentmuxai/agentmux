// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The macOS and Linux adapter for the input owner: turns CEF Views window
//! activation, pane lifecycle, the selection IPCs and (macOS) clicks into
//! machine events, and runs the machine's effects through three per-platform
//! functions (`platform`). docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §3.7.
//!
//! Compiled on Windows too, with stub platform functions, so the shared part is
//! built and checked there; only the macOS and Linux builds call it.

use std::sync::Arc;

use cef::*;

use crate::state::{AppState, BrowserPaneLifecycle};

use super::machine::{Effect, Event, Have, Owner, Surface};
use super::registry;

/// When a step's effects run. `Now` only on the UI thread, at a point where
/// focus may change at once (a click being dispatched); `Posted` everywhere
/// else, so an effect runs after the toolkit's own handling of the event.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Run {
    Now,
    Posted,
}

/// Step `window`'s machine and run what it returns; tell the window's page
/// when it changed (`input-owner-changed`).
pub fn dispatch(state: &Arc<AppState>, window: &str, event: Event, run: Run) {
    for effect in registry::step(window, event) {
        match (run, &effect) {
            // A check always waits its time.
            (_, Effect::Verify { .. }) | (Run::Posted, _) => post(state, window, effect),
            (Run::Now, _) => apply(state, window, effect),
        }
    }
    if let Some(report) = registry::report_if_changed(window) {
        crate::events::emit_event_to_window(state, window, "input-owner-changed", &report.to_json());
    }
}

// ── Events ───────────────────────────────────────────────────────────────

/// The frontend selected pane `block` (`input_select`, `browser_pane_focus`).
pub fn select_pane(state: &Arc<AppState>, block: &str) {
    match state.browser_pane_window_label(block) {
        Some(window) => dispatch(state, &window, Event::Select(Owner::Pane(block.to_string())), Run::Posted),
        None => tracing::warn!("[input] select of pane {block} with no window"),
    }
}

/// The frontend selected the app's own content in `window`.
pub fn select_app(state: &Arc<AppState>, window: &str) {
    dispatch(state, window, Event::Select(Owner::App), Run::Posted);
}

/// A click in pane `block`'s page, seen as it is dispatched (UI thread): the
/// page gets the keyboard before the click lands.
pub fn pane_pointer_down(state: &Arc<AppState>, block: &str) {
    if let Some(window) = state.browser_pane_window_label(block) {
        dispatch(state, &window, Event::PanePointerDown(block.to_string()), Run::Now);
    }
}

/// The user gave pane `block`'s page focus where no click hook sees it (Linux:
/// CEF's `on_set_focus` with a user source): the same as a click in it, posted.
/// Not while this adapter is focusing the pane itself, nor before the pane is
/// live: Views focuses a new pane's browser as it is added, and CEF may report
/// a focus set from code with the same source.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn pane_chosen(state: &Arc<AppState>, block: &str) {
    if ADAPTER_FOCUSING.with(|f| f.get()) {
        return;
    }
    let Some(window) = state.browser_pane_window_label(block) else { return };
    if !registry::pane_is_live(&window, block) {
        tracing::debug!("[input] win={window} focus on pane:{block} before it is live: not a choice");
        return;
    }
    dispatch(state, &window, Event::PanePointerDown(block.to_string()), Run::Posted);
}

thread_local! {
    /// Set while this adapter itself gives a pane browser focus, so CEF's
    /// report of that focus isn't taken for the user choosing the pane.
    static ADAPTER_FOCUSING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// CEF reported that pane `block`'s browser got focus.
pub fn pane_focus_gained(state: &Arc<AppState>, block: &str) {
    if let Some(window) = state.browser_pane_window_label(block) {
        dispatch(state, &window, Event::FocusGained(Surface::Pane(block.to_string())), Run::Posted);
    }
}

/// `window` became the active window, or stopped being it (CEF Views
/// `on_window_activation_changed`). On macOS a pane overlay is a key window of
/// its own, so the window is reported inactive whenever one of its own pages
/// takes the keyboard, and active again when the page gives it back; neither
/// is the user leaving or returning. Decided once the key change has settled
/// (posted):
/// - a deactivation counts only if neither the window nor one of its own pane
///   overlays is key;
/// - an activation of a window still counted active is the keyboard coming
///   back from its own page: focus arriving at the app, which doesn't ask for
///   the selection again (nor overrides a pane asked for since);
/// - a window whose page holds key hears nothing when another window takes
///   it, so an activation deactivates every other window still counted active
///   (`platform::others_active`): one window is key at a time.
pub fn window_activation(state: &Arc<AppState>, window: &str, active: bool) {
    let mut task = ActivationTask::new(state.clone(), window.to_string(), active);
    post_task(ThreadId::UI, Some(&mut task));
}

wrap_task! {
    struct ActivationTask {
        state: Arc<AppState>,
        window: String,
        active: bool,
    }
    impl Task {
        fn execute(&self) {
            let (state, window) = (&self.state, self.window.as_str());
            if !self.active {
                if platform::holds_key(state, window) {
                    tracing::debug!("[input] win={window} deactivation ignored: it or one of its panes is key");
                    return;
                }
                dispatch(state, window, Event::Deactivated, Run::Posted);
                return;
            }
            if platform::key_came_back(window) {
                // A hand-back the machine asked for may land after it has asked
                // for a pane since: then it is no news, and the pane's request
                // stands.
                if matches!(registry::snapshot(window), Some((_, Have::Pending { target: Owner::Pane(_), .. }, _))) {
                    tracing::debug!("[input] win={window} key came back while a pane is asked for; ignored");
                } else {
                    dispatch(state, window, Event::FocusGained(Surface::App), Run::Posted);
                }
                return;
            }
            for other in platform::others_active(window) {
                dispatch(state, &other, Event::Deactivated, Run::Posted);
            }
            dispatch(state, window, Event::Activated, Run::Posted);
        }
    }
}

/// Pane `block` can take focus now (Linux: its browser was created; macOS: its
/// overlay window is registered).
pub fn pane_live(state: &Arc<AppState>, block: &str) {
    if let Some(window) = state.browser_pane_window_label(block) {
        dispatch(state, &window, Event::PaneLive(block.to_string()), Run::Posted);
    }
}

/// Pane `block` was laid out in `window`: at 0×0 (its tab isn't the visible
/// one) or with a size.
pub fn pane_laid_out(state: &Arc<AppState>, window: &str, block: &str, collapsed: bool) {
    let event = if collapsed { Event::PaneCollapsed(block.to_string()) } else { Event::PaneShown(block.to_string()) };
    dispatch(state, window, event, Run::Posted);
}

/// Pane `block` is about to be destroyed.
pub fn pane_closing(state: &Arc<AppState>, block: &str) {
    if let Some(window) = registry::window_of_pane(block) {
        dispatch(state, &window, Event::PaneClosing(block.to_string()), Run::Posted);
    }
}

/// Pane `block` was destroyed.
pub fn pane_gone(state: &Arc<AppState>, block: &str) {
    if let Some(window) = registry::window_of_pane(block) {
        dispatch(state, &window, Event::PaneGone(block.to_string()), Run::Posted);
    }
}

// ── Effects ──────────────────────────────────────────────────────────────

fn apply(state: &Arc<AppState>, window: &str, effect: Effect) {
    match effect {
        // A posted focus effect runs only if it is still what the machine asks
        // for, in a window still active (`registry::still_requested`).
        Effect::FocusApp if registry::still_requested(window, &Owner::App) => platform::focus_app(state, window),
        Effect::FocusPane(block) if registry::still_requested(window, &Owner::Pane(block.clone())) => {
            platform::focus_pane(state, &block)
        }
        Effect::FocusApp | Effect::FocusPane(_) => {
            tracing::info!("[input] win={window} {effect} dropped: superseded or the window is inactive");
        }
        // Pane overlays are their own windows, ordered above the app's.
        Effect::RaisePanes => {}
        Effect::Verify { seq, .. } => {
            let surface = platform::focused(state, window);
            dispatch(state, window, Event::Verify { seq, surface }, Run::Posted);
        }
        Effect::Warn(reason) => tracing::warn!("[input] win={window} {reason}"),
    }
}

fn post(state: &Arc<AppState>, window: &str, effect: Effect) {
    let after_ms = match &effect {
        Effect::Verify { after_ms, .. } => *after_ms as i64,
        _ => 0,
    };
    let mut task = EffectTask::new(state.clone(), window.to_string(), effect);
    if after_ms > 0 {
        post_delayed_task(ThreadId::UI, Some(&mut task), after_ms);
    } else {
        post_task(ThreadId::UI, Some(&mut task));
    }
}

wrap_task! {
    struct EffectTask {
        state: Arc<AppState>,
        window: String,
        effect: Effect,
    }
    impl Task {
        fn execute(&self) {
            apply(&self.state, &self.window, self.effect.clone());
        }
    }
}

/// The browsers of `window`'s live panes.
fn panes_of(state: &Arc<AppState>, window: &str) -> Vec<Browser> {
    let labels: Vec<String> = state
        .host_state
        .lock()
        .browser_panes
        .values()
        .filter(|e| e.window_label == window && e.lifecycle == BrowserPaneLifecycle::Live)
        .map(|e| e.label.clone())
        .collect();
    labels.iter().filter_map(|l| state.get_browser(l)).collect()
}

/// `window`'s own browser takes focus at the Chromium level, and its panes
/// (only its own) give it up.
fn focus_window_browser(state: &Arc<AppState>, window: &str) {
    if let Some(host) = state.get_browser(window).and_then(|b| b.host()) {
        host.set_focus(1);
    }
    for pane in panes_of(state, window) {
        if let Some(host) = pane.host() {
            host.set_focus(0);
        }
    }
}

/// Pane `block`'s browser takes focus at the Chromium level.
fn focus_pane_browser(state: &Arc<AppState>, block: &str) {
    let host = state
        .live_browser_pane_label(block)
        .and_then(|l| state.get_browser(&l))
        .and_then(|b| b.host());
    if let Some(host) = host {
        ADAPTER_FOCUSING.with(|f| f.set(true));
        host.set_focus(1);
        ADAPTER_FOCUSING.with(|f| f.set(false));
    }
}

// ── Per platform ─────────────────────────────────────────────────────────

/// macOS: a pane has the keyboard while its overlay NSWindow is the key window
/// and its page is that window's first responder.
#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    /// `window`'s own NSWindow. Its Views window (`state.windows`) gives the
    /// content NSView, whose window it is.
    fn ns_window(state: &Arc<AppState>, window: &str) -> *mut std::ffi::c_void {
        let view = state
            .windows
            .lock()
            .get(window)
            .map(|w| w.window_handle() as *mut std::ffi::c_void)
            .unwrap_or(std::ptr::null_mut());
        if view.is_null() {
            return view;
        }
        crate::ui_tasks::ns_window_of_view(view)
    }

    pub fn focus_app(state: &Arc<AppState>, window: &str) {
        crate::ui_tasks::take_key_from_panes_of(ns_window(state, window));
        focus_window_browser(state, window);
    }

    pub fn focus_pane(state: &Arc<AppState>, block: &str) {
        focus_pane_browser(state, block);
        crate::ui_tasks::give_pane_overlay_key(block);
    }

    pub fn focused(state: &Arc<AppState>, window: &str) -> Surface {
        match crate::ui_tasks::key_surface_of(ns_window(state, window)) {
            Some(Some(block)) => Surface::Pane(block),
            Some(None) => Surface::App,
            None => Surface::Elsewhere,
        }
    }

    /// The window, or one of its own pane overlays, is the key window.
    pub fn holds_key(state: &Arc<AppState>, window: &str) -> bool {
        crate::ui_tasks::key_surface_of(ns_window(state, window)).is_some()
    }

    /// `window` became key while its machine still counts it active: its own
    /// page held the keyboard and gave it back (deactivations from that are
    /// ignored, `holds_key`).
    pub fn key_came_back(window: &str) -> bool {
        registry::is_active(window)
    }

    /// The other windows still counted active: on macOS only one is key.
    pub fn others_active(window: &str) -> Vec<String> {
        registry::active_windows().into_iter().filter(|w| w != window).collect()
    }

}

/// Linux: a pane is an overlay view inside its window's own widget, so the
/// keyboard follows Views focus there: the window's focus manager routes keys
/// to the focused view. Focus moves with `request_focus()` on the view (what a
/// click does) as well as Chromium's `set_focus`, and the check reads which
/// view has it (`has_focus()`). Unlike macOS, the overlay isn't a window of its
/// own, so focusing it doesn't touch window activation.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    /// `window`'s own BrowserView.
    fn window_view(state: &Arc<AppState>, window: &str) -> Option<BrowserView> {
        let mut browser = state.get_browser(window)?;
        browser_view_get_for_browser(Some(&mut browser))
    }

    /// The contents views of `window`'s panes, by block.
    fn pane_views(state: &Arc<AppState>, window: &str) -> Vec<(String, View)> {
        state
            .browser_pane_overlays
            .lock()
            .iter()
            .filter(|(_, (w, _))| w == window)
            .filter_map(|(label, (_, controller))| {
                let block = label.strip_prefix("browser-pane-")?.rsplit_once('-')?.0.to_string();
                Some((block, controller.contents_view()?))
            })
            .collect()
    }

    pub fn focus_app(state: &Arc<AppState>, window: &str) {
        if let Some(view) = window_view(state, window) {
            view.request_focus();
        }
        focus_window_browser(state, window);
    }

    pub fn focus_pane(state: &Arc<AppState>, block: &str) {
        let Some(window) = state.browser_pane_window_label(block) else { return };
        if let Some((_, view)) = pane_views(state, &window).into_iter().find(|(b, _)| b == block) {
            view.request_focus();
        }
        focus_pane_browser(state, block);
    }

    pub fn focused(state: &Arc<AppState>, window: &str) -> Surface {
        if let Some((block, _)) = pane_views(state, window).into_iter().find(|(_, v)| v.has_focus() != 0) {
            return Surface::Pane(block);
        }
        if window_view(state, window).is_some_and(|v| v.has_focus() != 0) {
            return Surface::App;
        }
        Surface::Elsewhere
    }

    /// Panes are views inside the window here: a deactivation is the window's.
    pub fn holds_key(_state: &Arc<AppState>, _window: &str) -> bool {
        false
    }

    pub fn key_came_back(_window: &str) -> bool {
        false
    }

    pub fn others_active(_window: &str) -> Vec<String> {
        Vec::new()
    }

}

/// Windows has its own adapter (`super::windows`); this only lets the shared
/// part above be built and checked there.
#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    pub fn focus_app(state: &Arc<AppState>, window: &str) {
        focus_window_browser(state, window);
    }

    pub fn focus_pane(state: &Arc<AppState>, block: &str) {
        focus_pane_browser(state, block);
    }

    pub fn focused(_state: &Arc<AppState>, _window: &str) -> Surface {
        Surface::Elsewhere
    }

    pub fn holds_key(_state: &Arc<AppState>, _window: &str) -> bool {
        false
    }

    pub fn key_came_back(_window: &str) -> bool {
        false
    }

    pub fn others_active(_window: &str) -> Vec<String> {
        Vec::new()
    }

}
