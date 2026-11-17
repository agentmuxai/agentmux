// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `/agentmux/browser/locate`: the visual fallback's hit test (forms-in-frames spec §4.6).
//!
//! For a region the accessibility tree can't describe (a canvas UI, the picture of a button, a control
//! whose accessible name is empty or wrong) the agent looks at a screenshot and names a point. This finds
//! what a click at that point would land on: `DOM.getNodeForLocation`, repeated in the frame's own session
//! when the point is over an `<iframe>` in another process, then up to the nearest element a person could
//! use (a link, a button, a field, something with a role or a click handler), which is given a reference
//! of its own in the pane's table. Nothing is clicked. The agent clicks the reference, so every rule a
//! reference click has applies (a committing click asks the person, the click must land on the element,
//! a frame off the pane's allowed sites is refused), and a field is filled by its reference, so the secret
//! guard always sees it.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::act::{call, describe, fail, page_url, unauthorized};
use super::cdp::CdpSession;
use super::frames::{self, FrameLayout, FrameRef};
use super::types::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct LocateReq {
    pub block_id: String,
    /// The point, in pixels of the screenshot the agent looked at.
    pub x: f64,
    pub y: f64,
    /// That screenshot's width in pixels; absent when the point is already in the page's own pixels.
    #[serde(default)]
    pub image_width: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct LocateData {
    /// The reference to click, fill, select or check the element with.
    #[serde(rename = "ref")]
    pub ref_: String,
    /// The element as a snapshot would show it: its role and accessible name.
    pub line: String,
    pub url: String,
    /// The site of the frame the element is in; absent for the page itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    /// The element is a secret field: it can be located, never read or filled.
    pub secret: bool,
}

/// The deepest `<iframe>` nesting followed, as in a snapshot.
const MAX_HOPS: usize = frames::MAX_FRAME_DEPTH + 1;

/// The nearest element at or above `this` that a person could use: a link with an address, a button, a
/// field, anything with an interactive role, a click handler or a tab stop. Failing that, the outermost
/// element of the unbroken run of pointer-cursor elements around it (a script-made click target: the
/// cursor is inherited, so the first one found is the leaf), never the page's body. `null` for plain text
/// or a canvas, which has nothing to take a reference by.
const NEAREST_CONTROL: &str = r#"function () {
  const up = (n) => n.parentElement || (n.getRootNode && n.getRootNode().host) || null;
  const ROLES = ["button", "link", "checkbox", "radio", "switch", "tab", "menuitem", "menuitemcheckbox",
    "menuitemradio", "option", "combobox", "textbox", "searchbox", "slider", "spinbutton", "treeitem"];
  const TAGS = ["BUTTON", "SELECT", "TEXTAREA", "SUMMARY", "OPTION", "LABEL"];
  const usable = (e) => {
    const t = e.tagName;
    if (t === "A") return e.hasAttribute("href");
    if (t === "INPUT") return String(e.type).toLowerCase() !== "hidden";
    if (TAGS.includes(t)) return true;
    if (ROLES.includes((e.getAttribute("role") || "").toLowerCase())) return true;
    if (e.getAttribute("contenteditable") !== null && e.isContentEditable) return true;
    if (e.hasAttribute("onclick")) return true;
    const stop = e.getAttribute("tabindex");
    return stop !== null && Number(stop) >= 0;
  };
  const start = this.nodeType === 1 ? this : up(this);
  for (let n = start, d = 0; n && d < 12; n = up(n), d++) {
    if (usable(n)) return n;
  }
  const pointer = (e) => getComputedStyle(e).cursor === "pointer";
  if (!start || !pointer(start)) return null;
  let best = start;
  for (let n = up(start), d = 0; n && d < 4 && pointer(n); n = up(n), d++) {
    if (n === n.ownerDocument.body || n === n.ownerDocument.documentElement) break;
    best = n;
  }
  return best === start.ownerDocument.body ? null : best;
}"#;

/// The point in the page's CSS pixels: a screenshot of the page is `image_width` pixels across for a page
/// `viewport_width` CSS pixels wide, whatever the display's density or the pane's zoom.
pub fn css_point(x: f64, y: f64, image_width: Option<f64>, viewport_width: f64) -> Result<(f64, f64), String> {
    let Some(w) = image_width else { return Ok((x, y)) };
    if !w.is_finite() || w < 1.0 || !viewport_width.is_finite() || viewport_width < 1.0 {
        return Err("image_width is the screenshot's width in pixels".to_string());
    }
    let scale = viewport_width / w;
    Ok((x * scale, y * scale))
}

/// The element as the agent reads it: `role "name"`, with only what is needed to tell it apart.
pub fn element_line(role: &str, name: &str, local_name: &str, secret: bool) -> String {
    let role = if role.is_empty() || role == "none" || role == "generic" { local_name } else { role };
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
    let mut line = if name.is_empty() { role.to_string() } else { format!("{role} {name:?}") };
    if secret {
        line.push_str(" [secret]");
    }
    line
}

/// The frame a node was found in, as a reference needs it; `None` for the page's own frame.
fn frame_ref(layout: &[FrameLayout], frame_id: &str) -> Option<FrameRef> {
    let l = layout.iter().find(|l| l.raw.id == frame_id)?;
    Some(FrameRef { frame_id: l.raw.id.clone(), session: l.raw.session.clone(), origin: l.origin.clone(), url: l.raw.url.clone(), chain: l.chain.clone() })
}

/// What is under `(x, y)`: the node, the frame it is in and the session that reaches it. A point over an
/// `<iframe>` whose page runs in another process is looked up again in that process, at the point
/// relative to the frame.
async fn hit_test(root: &mut CdpSession, layout: &[FrameLayout], x: f64, y: f64) -> Result<(i64, Option<String>, Option<String>), String> {
    let (mut session, mut px, mut py) = (None::<String>, x, y);
    for _ in 0..MAX_HOPS {
        let mut on = root.in_session(session.as_deref());
        let hit = call(
            &mut on,
            "DOM.getNodeForLocation",
            json!({ "x": px.round() as i64, "y": py.round() as i64, "includeUserAgentShadowDOM": false, "ignorePointerEventsNone": false }),
        )
        .await
        .map_err(|e| format!("nothing could be found at that point (is it outside the page?): {e}"))?;
        let backend = hit.get("backendNodeId").and_then(Value::as_i64).ok_or("nothing is at that point")?;
        let frame_id = hit.get("frameId").and_then(Value::as_str).map(str::to_string);
        // An `<iframe>` element names the frame it holds; one in another process is entered.
        let inner = on
            .call("DOM.describeNode", json!({ "backendNodeId": backend }))
            .await
            .ok()
            .and_then(|v| v.pointer("/node/frameId").and_then(Value::as_str).map(str::to_string));
        let entered = inner.as_deref().and_then(|id| layout.iter().find(|l| l.raw.id == id && l.is_session_root));
        match entered {
            Some(l) => {
                let (ox, oy) = frames::chain_offset(root, &l.chain).await?;
                session = l.raw.session.clone();
                px -= ox;
                py -= oy;
            }
            None => return Ok((backend, frame_id, session)),
        }
    }
    Err("that point is inside frames nested deeper than a snapshot reads".to_string())
}

/// `POST /agentmux/browser/locate`.
pub async fn locate_route(State(state): State<Arc<AppState>>, headers: HeaderMap, Json(req): Json<LocateReq>) -> (StatusCode, Json<ApiResponse<LocateData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    if !req.x.is_finite() || !req.y.is_finite() || req.x < 0.0 || req.y < 0.0 {
        return fail("x and y are positions in the page, counted from its top left corner");
    }
    // Before the pane is opened, so a close any time after is seen when the reference is kept.
    let epoch = state.browser_api.ref_tables.epoch(&req.block_id);
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "locate").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = locate(&state, &mut cdp, &req, epoch).await;
    let _ = cdp.close().await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::ok(data))),
        Err(e) => fail(e),
    }
}

async fn locate(state: &Arc<AppState>, root: &mut CdpSession, req: &LocateReq, epoch: u64) -> Result<LocateData, String> {
    let _ = root.call("Accessibility.enable", json!({})).await;
    let layout = frames::layout(root).await;
    let viewport = root
        .call("Runtime.evaluate", json!({ "expression": "window.innerWidth", "returnByValue": true }))
        .await
        .ok()
        .and_then(|v| v.pointer("/result/value").and_then(Value::as_f64))
        .unwrap_or(0.0);
    let (x, y) = css_point(req.x, req.y, req.image_width, viewport)?;
    let (backend, frame_id, session) = hit_test(root, &layout, x, y).await?;
    let frame = frame_id.as_deref().and_then(|id| frame_ref(&layout, id));
    if let Some(f) = &frame {
        // A frame off the pane's allowed sites is shown but never acted on (spec §5.3).
        let allowed = state.site_limits.list(&req.block_id);
        frames::refuse_if_blocked(root, allowed.as_deref(), f, "the element at that point").await?;
    }
    let mut on = root.in_session(session.as_deref());
    let ctx = super::act::isolated_context(&mut on, frame_id.as_deref()).await?;
    let obj = super::act::resolve(&mut on, backend, ctx, "the element at that point").await?;
    // The element itself, not its value: the answer is an object, whose node id is the reference's.
    let found = on
        .call("Runtime.callFunctionOn", json!({ "objectId": obj, "functionDeclaration": NEAREST_CONTROL, "returnByValue": false }))
        .await
        .map_err(|e| format!("CDP Runtime.callFunctionOn: {e}"))?;
    let target = found
        .pointer("/result/objectId")
        .and_then(Value::as_str)
        .ok_or("nothing at that point can be clicked or filled by reference (plain text, a picture or a canvas): hand the step to the person with handoff")?
        .to_string();
    let described = call(&mut on, "DOM.describeNode", json!({ "objectId": target })).await?;
    let control = described.pointer("/node/backendNodeId").and_then(Value::as_i64).ok_or("the element at that point has no node to take a reference by")?;
    let facts = describe(&mut on, control).await.ok_or("the element at that point is gone from the page")?;
    let role = on
        .call("Accessibility.getPartialAXTree", json!({ "backendNodeId": control, "fetchRelatives": false }))
        .await
        .ok()
        .and_then(|t| t.pointer("/nodes/0/role/value").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let secret = super::secret::is_secret(&facts);
    let url = page_url(root).await;
    let name = format!("{}e1", state.browser_api.ref_tables.next_find_prefix(&req.block_id));
    let framed: Vec<(String, FrameRef)> = frame.iter().map(|f| (name.clone(), f.clone())).collect();
    if !state.browser_api.ref_tables.merge_framed_at(&req.block_id, Some(epoch), &url, &[(name.clone(), control)], &framed) {
        return Err("the page closed while this was being read; take a new snapshot".to_string());
    }
    Ok(LocateData { ref_: name, line: element_line(&role, &facts.label, &facts.local_name, secret), url, frame: frame.map(|f| f.origin), secret })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screenshot_pixel_is_scaled_by_the_pages_width_over_the_images() {
        // A 2x display: a 1920 px image of a 960 px page.
        assert_eq!(css_point(200.0, 100.0, Some(1920.0), 960.0).unwrap(), (100.0, 50.0));
        // A zoomed-out pane: a 800 px image of a 1600 px page.
        assert_eq!(css_point(100.0, 40.0, Some(800.0), 1600.0).unwrap(), (200.0, 80.0));
        // No width: the point is the page's own.
        assert_eq!(css_point(12.0, 34.0, None, 0.0).unwrap(), (12.0, 34.0));
    }

    #[test]
    fn a_width_that_cannot_scale_anything_is_refused() {
        assert!(css_point(1.0, 1.0, Some(0.0), 960.0).is_err());
        assert!(css_point(1.0, 1.0, Some(800.0), 0.0).is_err(), "a page whose width is unknown can't be scaled to");
        assert!(css_point(1.0, 1.0, Some(f64::NAN), 960.0).is_err());
    }

    #[test]
    fn the_element_is_read_as_its_role_and_name() {
        assert_eq!(element_line("button", "Next", "button", false), "button \"Next\"");
        assert_eq!(element_line("textbox", "  Full   name ", "input", false), "textbox \"Full name\"");
    }

    #[test]
    fn an_element_with_no_role_is_read_by_its_tag_and_a_nameless_one_has_no_name() {
        assert_eq!(element_line("generic", "", "div", false), "div");
        assert_eq!(element_line("", "Pay", "div", false), "div \"Pay\"");
        assert_eq!(element_line("none", "", "img", false), "img");
    }

    #[test]
    fn a_secret_field_is_marked_and_a_long_name_is_cut() {
        assert_eq!(element_line("textbox", "Taxpayer TIN", "input", true), "textbox \"Taxpayer TIN\" [secret]");
        let long = "x".repeat(300);
        assert_eq!(element_line("button", &long, "button", false).chars().count(), "button \"\"".len() + 80);
    }

    #[test]
    fn a_frame_is_found_by_its_id_and_the_page_itself_has_none() {
        let raw = frames::RawFrame { id: "F1".into(), parent_id: Some("T".into()), url: "https://pay.example/f".into(), origin: "https://pay.example".into(), session: Some("s1".into()) };
        let layout = vec![FrameLayout { ordinal: 1, parent: 0, depth: 1, raw, owner: Some(9), chain: Vec::new(), is_session_root: true, origin: "https://pay.example".into(), cross_origin: true, placeable: true }];
        let f = frame_ref(&layout, "F1").unwrap();
        assert_eq!((f.frame_id.as_str(), f.session.as_deref(), f.origin.as_str()), ("F1", Some("s1"), "https://pay.example"));
        assert!(frame_ref(&layout, "TOP").is_none());
    }
}
