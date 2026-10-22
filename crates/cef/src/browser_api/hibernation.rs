// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a browser pane's page is doing that a hibernation would lose, asked by
//! the window before it puts a hidden pane to sleep
//! (docs/specs/SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md §5.3): media
//! playing, a form field the person changed, a `beforeunload` handler, DevTools
//! open. Read in an isolated world, so the page can't answer for itself; the
//! `beforeunload` listeners come from `DOMDebugger.getEventListeners` on the
//! page's `window`.

use std::sync::Arc;

use serde_json::{json, Value};

use cef::{ImplBrowser, ImplBrowserHost};

use super::act::isolated_context;
use super::cdp::CdpSession;
use crate::state::AppState;

/// Media playing, and form fields changed from their defaults, in the main
/// frame. A focused, non-empty rich-text editor counts as edited: its default
/// isn't recorded anywhere a check can read.
const PAGE_CHECK: &str = r#"(() => {
  const media = [...document.querySelectorAll("video, audio")].some(
    (m) => !m.paused && !m.ended && m.readyState > 2
  );
  const changed = (el) => {
    if (el.disabled || el.type === "hidden") return false;
    if (el.tagName === "SELECT") return [...el.options].some((o) => o.selected !== o.defaultSelected);
    if (el.type === "checkbox" || el.type === "radio") return el.checked !== el.defaultChecked;
    return typeof el.value === "string" && el.value !== el.defaultValue;
  };
  let edited = [...document.querySelectorAll("input, textarea, select")].some(changed);
  const active = document.activeElement;
  if (!edited && active && active.isContentEditable && (active.textContent || "").trim()) edited = true;
  return { media, edited };
})()"#;

/// The page's state for the hibernation check. `live: false` when the pane has
/// no page (already asleep, or closed).
pub async fn check(state: &Arc<AppState>, block_id: &str) -> Result<Value, String> {
    let Some(label) = state.live_browser_pane_label(block_id) else {
        return Ok(json!({ "live": false }));
    };
    let devtools = state
        .get_browser(&label)
        .and_then(|b| b.host())
        .is_some_and(|h| h.has_dev_tools() != 0);
    let mut cdp = CdpSession::attach(state, &label);
    let ctx = isolated_context(&mut cdp, None).await?;
    let page = cdp
        .call("Runtime.evaluate", json!({ "expression": PAGE_CHECK, "contextId": ctx, "returnByValue": true }))
        .await?;
    let facts = page.pointer("/result/value").cloned().unwrap_or(Value::Null);
    let beforeunload = has_beforeunload(&mut cdp).await;
    cdp.close().await;
    Ok(json!({
        "live": true,
        "media": facts.get("media").and_then(Value::as_bool).unwrap_or(false),
        "edited": facts.get("edited").and_then(Value::as_bool).unwrap_or(false),
        "beforeunload": beforeunload,
        "devtools": devtools,
    }))
}

/// Freeze the pane's page (`Page.setWebLifecycleState` frozen: no script, no
/// timers, its state kept) or let it run again. The window freezes a pane hidden
/// for a while and thaws it when shown; any other CDP call thaws it too
/// (`cdp::CdpSession::call`), so an agent's action never waits on it. False
/// when the pane has no page.
pub async fn set_frozen(state: &Arc<AppState>, block_id: &str, frozen: bool) -> Result<Value, String> {
    let Some(label) = state.live_browser_pane_label(block_id) else {
        return Ok(json!(false));
    };
    let Some(browser_id) = state.get_browser(&label).map(|b| b.identifier()) else {
        return Ok(json!(false));
    };
    let mut cdp = CdpSession::attach(state, &label);
    let state_name = if frozen { "frozen" } else { "active" };
    cdp.call(super::cdp::LIFECYCLE_METHOD, json!({ "state": state_name })).await?;
    super::cdp::note_frozen(browser_id, frozen);
    Ok(json!(true))
}

/// Whether the page's `window` has a `beforeunload` listener (added with
/// `addEventListener` or as `onbeforeunload`). Unknown counts as yes: a check
/// that couldn't run must keep the pane awake.
async fn has_beforeunload(cdp: &mut CdpSession) -> bool {
    const GROUP: &str = "agentmux-hibernation";
    let Ok(w) = cdp
        .call("Runtime.evaluate", json!({ "expression": "window", "objectGroup": GROUP }))
        .await
    else {
        return true;
    };
    let Some(object_id) = w.pointer("/result/objectId").and_then(Value::as_str).map(str::to_string) else {
        return true;
    };
    let listeners = cdp.call("DOMDebugger.getEventListeners", json!({ "objectId": object_id })).await;
    let _ = cdp.call("Runtime.releaseObjectGroup", json!({ "objectGroup": GROUP })).await;
    match listeners {
        Ok(l) => any_beforeunload(&l),
        Err(_) => true,
    }
}

fn any_beforeunload(listeners: &Value) -> bool {
    listeners
        .get("listeners")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|l| l.get("type").and_then(Value::as_str) == Some("beforeunload")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_beforeunload_listener_among_others() {
        assert!(any_beforeunload(&json!({ "listeners": [{ "type": "load" }, { "type": "beforeunload" }] })));
        assert!(!any_beforeunload(&json!({ "listeners": [{ "type": "load" }] })));
        assert!(!any_beforeunload(&json!({})));
    }
}
