// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a browser pane's page is doing that a hibernation would lose, asked by
//! the window before it puts a hidden pane to sleep
//! (docs/specs/SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md §5.3): media
//! playing, a form field the person changed, a `beforeunload` handler, DevTools
//! open. Read in an isolated world, so the page can't answer for itself; the
//! `beforeunload` listeners come from `DOMDebugger.getEventListeners` on the
//! page's `window`.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{json, Value};

use cef::{ImplBrowser, ImplBrowserHost};

use super::act::isolated_context;
use super::cdp::CdpSession;
use crate::state::AppState;

/// Media playing, and text typed into the main frame's fields: a text field or
/// textarea whose value differs from its default. Checkboxes, radio buttons
/// and selects don't count: pages set them from script (Wikipedia ticks its
/// theme and width radios on load), and losing one is a click, not lost
/// writing. A focused, non-empty rich-text editor counts as edited: its default
/// isn't recorded anywhere a check can read.
const PAGE_CHECK: &str = r#"(() => {
  const media = [...document.querySelectorAll("video, audio")].some(
    (m) => !m.paused && !m.ended && m.readyState > 2
  );
  const notText = new Set(["hidden", "checkbox", "radio", "button", "submit", "reset", "image", "file", "range", "color"]);
  const typed = (el) =>
    !el.disabled && !el.readOnly && !notText.has(el.type) && typeof el.value === "string" && el.value !== el.defaultValue;
  let edited = [...document.querySelectorAll("input, textarea")].some(typed);
  const active = document.activeElement;
  if (!edited && active && active.isContentEditable && (active.textContent || "").trim()) edited = true;
  // Chrome shows "Leave site?" only to a page its person has interacted with.
  const activated = navigator.userActivation ? navigator.userActivation.hasBeenActive : true;
  return { media, edited, activated };
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
    let activated = facts.get("activated").and_then(Value::as_bool).unwrap_or(true);
    cdp.close().await;
    Ok(json!({
        "live": true,
        "media": facts.get("media").and_then(Value::as_bool).unwrap_or(false),
        "edited": facts.get("edited").and_then(Value::as_bool).unwrap_or(false),
        "beforeunload": beforeunload,
        "leave_prompt": leave_prompt(beforeunload, activated),
        "devtools": devtools,
    }))
}

/// `browser_pane_memory { block_ids }`: the memory each of those browser panes
/// holds in its own renderers, in bytes, by block id (all panes when none are
/// named). A pane whose figure is unknown is left out. Not on Linux: the renderer
/// map holds sandbox-namespace PIDs there, which only srv translates, so a match
/// against host PIDs could name another process.
#[cfg(not(target_os = "linux"))]
pub async fn pane_memory(state: &Arc<AppState>, args: &Value) -> Result<Value, String> {
    let only: Vec<String> = args
        .get("block_ids")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let entries = crate::renderer_map::snapshot(state);
    let procs = tokio::task::spawn_blocking(agentmux_procstats::snapshot)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(json!(crate::renderer_map::pane_memory(&entries, &procs, &only)))
}

#[cfg(target_os = "linux")]
pub async fn pane_memory(_state: &Arc<AppState>, _args: &Value) -> Result<Value, String> {
    Ok(json!({}))
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

/// Whether any frame of the page has a `beforeunload` listener: closing the
/// pane asks every frame, not only the top one (an Azure portal blade is a
/// cross-origin iframe that holds its own). Unknown counts as yes: a check that
/// couldn't run, or a frame it can't read, must keep the pane awake and make
/// closing it ask.
async fn has_beforeunload(cdp: &mut CdpSession) -> bool {
    let (frames, targets) = super::frames::current_frames(cdp).await;
    let roots: HashSet<&str> = targets.iter().map(|t| t.target_id.as_str()).collect();
    if has_unreadable_frame(&frames, &roots) {
        return true;
    }
    // The page's own process, then each frame running in a process of its own.
    if window_has_beforeunload(cdp).await {
        return true;
    }
    for t in &targets {
        let mut on = cdp.in_session(Some(&t.session_id));
        if window_has_beforeunload(&mut on).await {
            return true;
        }
    }
    false
}

/// A frame in a page's own process, from another origin than the frame that
/// holds it, can't be read from outside (its window is a cross-origin proxy).
/// A frame in its own process (`roots`) is read through its own session instead.
fn has_unreadable_frame(frames: &[super::frames::RawFrame], roots: &HashSet<&str>) -> bool {
    frames.iter().any(|f| {
        let Some(parent) = f.parent_id.as_deref().and_then(|p| frames.iter().find(|x| x.id == p)) else {
            return false;
        };
        !roots.contains(f.id.as_str()) && f.origin != parent.origin
    })
}

/// Whether this process's main frame, or a same-origin frame inside it (found
/// through `window.frames`), has a `beforeunload` listener (added with
/// `addEventListener` or as `onbeforeunload`).
async fn window_has_beforeunload(cdp: &mut CdpSession) -> bool {
    const GROUP: &str = "agentmux-hibernation";
    let Ok(w) = cdp
        .call("Runtime.evaluate", json!({ "expression": "window", "objectGroup": GROUP }))
        .await
    else {
        return true;
    };
    let Some(top) = w.pointer("/result/objectId").and_then(Value::as_str).map(str::to_string) else {
        return true;
    };
    let mut found = false;
    let mut stack = vec![(top, 0usize)];
    let mut seen = 0usize;
    while let Some((object_id, depth)) = stack.pop() {
        seen += 1;
        if seen > super::frames::MAX_FRAMES {
            found = true;
            break;
        }
        match cdp.call("DOMDebugger.getEventListeners", json!({ "objectId": object_id })).await {
            Ok(l) if any_beforeunload(&l) => found = true,
            Ok(_) => {}
            Err(_) => found = true,
        }
        if found {
            break;
        }
        if depth >= super::frames::MAX_FRAME_DEPTH {
            continue;
        }
        let count = cdp
            .call(
                "Runtime.callFunctionOn",
                json!({ "objectId": object_id, "functionDeclaration": "function () { return this.length; }", "returnByValue": true }),
            )
            .await
            .ok()
            .and_then(|r| r.pointer("/result/value").and_then(Value::as_u64));
        let Some(count) = count else {
            found = true;
            break;
        };
        for i in 0..count {
            // A frame this window can't read (another origin, or its own process)
            // answers null: those are covered by the frame check and the sessions.
            let child = cdp
                .call(
                    "Runtime.callFunctionOn",
                    json!({
                        "objectId": object_id,
                        "functionDeclaration": "function (i) { try { void this.frames[i].document; return this.frames[i]; } catch (e) { return null; } }",
                        "arguments": [{ "value": i }],
                        "objectGroup": GROUP,
                    }),
                )
                .await;
            if let Some(id) = child.ok().and_then(|r| r.pointer("/result/objectId").and_then(Value::as_str).map(str::to_string)) {
                stack.push((id, depth + 1));
            }
        }
    }
    let _ = cdp.call("Runtime.releaseObjectGroup", json!({ "objectGroup": GROUP })).await;
    found
}

/// Whether closing the pane would show the page's "Leave site?": a
/// `beforeunload` listener, and the interaction Chrome requires before it lets
/// one prompt. The window asks the person first when it is true
/// (`pane-close-guard.ts`). A page can still answer without a prompt, so this
/// errs towards asking.
fn leave_prompt(beforeunload: bool, activated: bool) -> bool {
    beforeunload && activated
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

    fn frame(id: &str, parent: Option<&str>, origin: &str) -> super::super::frames::RawFrame {
        super::super::frames::RawFrame {
            id: id.into(),
            parent_id: parent.map(str::to_string),
            url: format!("{origin}/"),
            origin: origin.into(),
            session: None,
        }
    }

    #[test]
    fn a_cross_origin_frame_in_the_pages_own_process_cannot_be_read() {
        let frames = [frame("top", None, "https://a.test"), frame("kid", Some("top"), "https://b.a.test")];
        assert!(has_unreadable_frame(&frames, &HashSet::new()));
    }

    #[test]
    fn a_frame_in_its_own_process_is_read_through_its_session() {
        let frames = [frame("top", None, "https://a.test"), frame("blade", Some("top"), "https://b.test")];
        assert!(!has_unreadable_frame(&frames, &HashSet::from(["blade"])));
    }

    #[test]
    fn same_origin_frames_and_a_lone_page_are_readable() {
        let frames = [
            frame("top", None, "https://a.test"),
            frame("kid", Some("top"), "https://a.test"),
            frame("grandkid", Some("kid"), "https://a.test"),
        ];
        assert!(!has_unreadable_frame(&frames, &HashSet::new()));
        assert!(!has_unreadable_frame(&frames[..1], &HashSet::new()));
    }

    #[test]
    fn a_leave_prompt_needs_a_listener_and_an_interaction() {
        assert!(leave_prompt(true, true));
        assert!(!leave_prompt(true, false), "Chrome shows no prompt to a page nobody has touched");
        assert!(!leave_prompt(false, true));
        assert!(!leave_prompt(false, false));
    }

    #[test]
    fn finds_a_beforeunload_listener_among_others() {
        assert!(any_beforeunload(&json!({ "listeners": [{ "type": "load" }, { "type": "beforeunload" }] })));
        assert!(!any_beforeunload(&json!({ "listeners": [{ "type": "load" }] })));
        assert!(!any_beforeunload(&json!({})));
    }
}
