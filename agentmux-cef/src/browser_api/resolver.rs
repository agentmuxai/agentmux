// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Resolve `block_id` → the CEF `Browser` (by host-state label) whose page
//! holds it.
//!
//! The browser API drives CDP in-process through each `Browser`'s own
//! DevTools message channel (`super::cdp`), so a target is simply a
//! `Browser` — which host state already tracks by label. That replaced the
//! old approach of listing the remote-debugging server's `/json` targets
//! and matching by URL (then by a stamped page when several panes shared a
//! URL): with the browser in hand there is nothing to match (#3681).
//!
//! **Path 1 — dedicated browser-pane block** (a `view: "browser"` widget,
//! embedding its own separate CEF `Browser` for arbitrary third-party
//! content): the pane's own live browser, straight from host state.
//!
//! **Path 2 — any other block** (agent/terminal/editor/etc. panes, which
//! are DOM nodes inside a SHARED main/pool/floating window's page, not their
//! own browser — see `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`
//! §1): ask each top-level window's page (one `Runtime.evaluate`) whether its
//! DOM contains a `[data-blockid="<id>"]` element (set on every pane's root
//! wrapper by `frontend/app/block/blockframe.tsx`). First match wins. Only
//! top-level app windows and floaters are probed (`list_top_level_browsers`)
//! — never a browser pane's third-party page, which could otherwise plant a
//! matching element, and never an OAuth popup.
//!
//! Callers MUST know which path resolved, because it changes whether
//! DOM-subtree scoping is required: a Path-2 page is SHARED (other panes'
//! DOM lives there too — queries/clicks/screenshots must scope to
//! `[data-blockid]`), while a Path-1 page already IS the block's own
//! isolated page (arbitrary third-party content with no `data-blockid`
//! concept — scoping would break it). See `ResolvedTarget`.
//!
//! Cache: Path 2 remembers which window last held a block and probes it
//! first, but always re-probes — a block can move windows (tear-off,
//! re-dock), and a probe is one in-process evaluate. Path 1 needs no cache.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use parking_lot::Mutex;

use super::cdp::CdpSession;
use crate::state::AppState;

pub type ResolveError = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    /// Host-state label of the `Browser` to drive (`super::cdp::CdpSession::attach`).
    pub label: String,
    /// True when this browser's page is SHARED with other panes (the main
    /// AgentMux UI, or a pool/floating window) — callers must scope
    /// queries/clicks/screenshots to this block's own `[data-blockid]`
    /// subtree. False for a dedicated browser pane, whose page already IS
    /// this block's own.
    pub scope_to_block: bool,
}

#[derive(Default)]
pub struct TargetCache {
    // block_id → window label that last held it (Path 2 only)
    last_window: Mutex<HashMap<String, String>>,
}

impl TargetCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn resolve(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
    ) -> Result<ResolvedTarget, ResolveError> {
        // Path 1: a live browser pane is its own browser.
        if let Some(label) = state.live_browser_pane_label(block_id) {
            return Ok(ResolvedTarget {
                label,
                scope_to_block: false,
            });
        }

        // Path 2: which top-level window's page holds this block? Labels
        // only — `Browser` handles aren't Send and aren't needed here.
        let windows = path2_candidates(
            state.list_top_level_browsers().into_iter().map(|(label, _)| label),
            |label| state.is_subwindow(label),
        );
        let cached = self.last_window.lock().get(block_id).cloned();
        let expr = block_probe_expr(block_id);
        let label = find_window_holding(block_id, &windows, cached.as_deref(), |label| {
            let mut cdp = CdpSession::attach(state, label);
            let expr = expr.clone();
            async move {
                cdp.call(
                    "Runtime.evaluate",
                    serde_json::json!({ "expression": expr, "returnByValue": true }),
                )
                .await
                .ok()
                .and_then(|v| v.get("result").and_then(|r| r.get("value")).cloned())
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            }
        })
        .await?;
        self.last_window.lock().insert(block_id.to_string(), label.clone());
        Ok(ResolvedTarget {
            label,
            scope_to_block: true,
        })
    }

    /// Drop a block's remembered window — called when a pane closes or
    /// navigates. The next resolve probes every window.
    pub fn forget(&self, block_id: &str) {
        self.last_window.lock().remove(block_id);
    }
}

/// Probe `windows` (the remembered one first, if still open) and return the
/// first whose page holds `block_id`. Deliberately NOT exclusive: many
/// blocks legitimately share one window, so a window that already resolved
/// another block stays a candidate (the exclusivity bug caught in live
/// verification on 2026-08-19 must not come back —
/// `many_blocks_can_share_one_window`).
async fn find_window_holding<F, Fut>(
    block_id: &str,
    windows: &[String],
    cached: Option<&str>,
    mut holds_block: F,
) -> Result<String, ResolveError>
where
    F: FnMut(&str) -> Fut,
    Fut: Future<Output = bool>,
{
    for label in probe_order(windows, cached) {
        if holds_block(&label).await {
            return Ok(label);
        }
    }
    Err(format!(
        "UNKNOWN_BLOCK_ID: no window's DOM contains a [data-blockid=\"{block_id}\"] element \
         (probed {} windows)",
        windows.len()
    ))
}

/// Top-level windows a Path-2 block may live in: every app window and
/// floater except approval subwindows, excluded by KIND so an approval page
/// is structurally unreachable rather than merely lacking a matching
/// `[data-blockid]` (Opaz's review of #3832).
fn path2_candidates(
    top_level: impl IntoIterator<Item = String>,
    is_subwindow: impl Fn(&str) -> bool,
) -> Vec<String> {
    top_level.into_iter().filter(|label| !is_subwindow(label)).collect()
}

/// `windows` with `cached` moved to the front — only if it is still open.
fn probe_order(windows: &[String], cached: Option<&str>) -> Vec<String> {
    let mut order: Vec<String> = Vec::with_capacity(windows.len());
    if let Some(c) = cached.filter(|c| windows.iter().any(|w| w == c)) {
        order.push(c.to_string());
    }
    order.extend(windows.iter().filter(|w| Some(w.as_str()) != cached).cloned());
    order
}

/// JS answering whether this page holds `block_id`'s pane. `block_id` is
/// JSON-encoded into a string literal and then `CSS.escape`d inside the
/// page, so no block id can break out of the selector.
fn block_probe_expr(block_id: &str) -> String {
    let block_id_js = serde_json::to_string(block_id).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "(() => {{ try {{ return !!document.querySelector(\
         '[data-blockid=\"' + CSS.escape({block_id_js}) + '\"]'); \
         }} catch (e) {{ return false; }} }})()"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn labels(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// A fake probe: `layout` maps window → the blocks its page holds.
    fn probe<'a>(
        layout: &'a HashMap<&'static str, HashSet<&'static str>>,
        block: &'static str,
    ) -> impl FnMut(&str) -> std::future::Ready<bool> + 'a {
        move |w: &str| std::future::ready(layout.get(w).is_some_and(|b| b.contains(block)))
    }

    #[tokio::test]
    async fn many_blocks_can_share_one_window() {
        let layout = HashMap::from([("main", HashSet::from(["a", "b", "c"]))]);
        let windows = labels(&["main"]);
        for block in ["a", "b", "c"] {
            let got = find_window_holding(block, &windows, None, probe(&layout, block)).await;
            assert_eq!(got.unwrap(), "main", "block {block} must resolve even after others claimed main");
        }
    }

    #[tokio::test]
    async fn finds_the_window_a_block_lives_in() {
        let layout = HashMap::from([
            ("main", HashSet::from(["a"])),
            ("floating-1", HashSet::from(["torn-off"])),
        ]);
        let windows = labels(&["main", "floating-1"]);
        let got = find_window_holding("torn-off", &windows, None, probe(&layout, "torn-off")).await;
        assert_eq!(got.unwrap(), "floating-1");
    }

    #[tokio::test]
    async fn a_stale_remembered_window_is_re_probed_not_trusted() {
        // The block was in main, then torn off to floating-1.
        let layout = HashMap::from([
            ("main", HashSet::from([])),
            ("floating-1", HashSet::from(["moved"])),
        ]);
        let windows = labels(&["main", "floating-1"]);
        let got = find_window_holding("moved", &windows, Some("main"), probe(&layout, "moved")).await;
        assert_eq!(got.unwrap(), "floating-1");
    }

    #[tokio::test]
    async fn nothing_matching_is_a_clear_error() {
        let layout = HashMap::from([("main", HashSet::from(["a"]))]);
        let err = find_window_holding("ghost", &labels(&["main"]), None, probe(&layout, "ghost"))
            .await
            .unwrap_err();
        assert!(err.starts_with("UNKNOWN_BLOCK_ID"), "{err}");
        assert!(err.contains("ghost") && err.contains("probed 1 windows"), "{err}");
    }

    #[test]
    fn approval_subwindows_are_never_path2_candidates() {
        let subwindows = ["window-approval-1"];
        let got = path2_candidates(
            labels(&["main", "window-approval-1", "floating-1"]),
            |l| subwindows.contains(&l),
        );
        assert_eq!(got, labels(&["main", "floating-1"]));
    }

    #[test]
    fn the_remembered_window_is_probed_first_only_while_it_is_open() {
        let w = labels(&["main", "floating-1", "window-2"]);
        assert_eq!(probe_order(&w, Some("floating-1")), labels(&["floating-1", "main", "window-2"]));
        assert_eq!(probe_order(&w, Some("closed-window")), w);
        assert_eq!(probe_order(&w, None), w);
    }

    #[test]
    fn the_probe_escapes_the_block_id_inside_the_page() {
        let expr = block_probe_expr(r#"x"]');alert(1)//"#);
        // The id only ever appears as a JSON string literal passed to CSS.escape.
        assert!(expr.contains(r#"CSS.escape("x\"]');alert(1)//")"#), "{expr}");
        assert!(!expr.contains(r#"[data-blockid="x"]"#));
    }
}
