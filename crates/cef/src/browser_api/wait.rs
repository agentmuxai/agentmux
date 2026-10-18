// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `/agentmux/browser/wait_for`: wait until the page shows some text, reaches
//! an address, or an element leaves it. Text is looked for in the top frame
//! and in every iframe, each through the session that reaches it: a form
//! that lives in an iframe is still "on the page" to the person reading it.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::act::{describe_markup, fail, isolated_context, page_url, unauthorized};
use super::cdp::CdpSession;
use super::frames::{self, FrameRef, RawFrame};
use super::types::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct WaitForReq {
    pub block_id: String,
    /// Wait until the page's visible text, in any frame, contains this.
    #[serde(default)]
    pub text: Option<String>,
    /// Wait until the page URL contains this.
    #[serde(default)]
    pub url_contains: Option<String>,
    /// Wait until the element behind this reference has left the page.
    #[serde(default)]
    pub gone: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct WaitForData {
    pub waited_ms: u64,
    pub url: String,
    /// The address of the iframe the text was found in; absent when it was
    /// in the top frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
}

/// Run in an isolated world with the text as its argument, so the page's own
/// scripts can't answer for it.
const HAS_TEXT: &str = "((t) => !!(document.body && document.body.innerText.includes(t)))";

/// The isolated worlds made during one wait, by session and frame: a wait of
/// up to a minute looks four times a second, and needs one world per frame,
/// not one per look.
type Worlds = HashMap<(Option<String>, String), i64>;

/// Does `frame`'s visible text contain `text`? A world that went away with a
/// navigation is made again, once.
async fn frame_has_text(root: &CdpSession, frame: &RawFrame, text: &str, worlds: &mut Worlds) -> bool {
    let key = (frame.session.clone(), frame.id.clone());
    let mut on = root.in_session(frame.session.as_deref());
    let expression = format!("{HAS_TEXT}({})", serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into()));
    for _ in 0..2 {
        let ctx = match worlds.get(&key) {
            Some(c) => *c,
            None => match isolated_context(&mut on, Some(&frame.id)).await {
                Ok(c) => *worlds.entry(key.clone()).or_insert(c),
                Err(_) => return false,
            },
        };
        let reply = on
            .call("Runtime.evaluate", json!({ "expression": expression, "contextId": ctx, "returnByValue": true }))
            .await;
        if let Some(found) = reply
            .ok()
            .filter(|v| v.get("exceptionDetails").is_none())
            .and_then(|v| v.pointer("/result/value").and_then(Value::as_bool))
        {
            return found;
        }
        worlds.remove(&key);
    }
    false
}

/// The frame showing `text`, if any: `Some(None)` for the top frame,
/// `Some(Some(url))` for an iframe.
async fn find_text(root: &mut CdpSession, text: &str, worlds: &mut Worlds) -> Option<Option<String>> {
    let (all, _) = frames::current_frames(root).await;
    for (i, f) in all.iter().enumerate() {
        if frame_has_text(root, f, text, worlds).await {
            return Some((i > 0).then(|| f.url.clone()));
        }
    }
    None
}

/// `POST /agentmux/browser/wait_for`.
pub async fn wait_for_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<WaitForReq>,
) -> (StatusCode, Json<ApiResponse<WaitForData>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let conditions = [req.text.is_some(), req.url_contains.is_some(), req.gone.is_some()];
    if conditions.iter().filter(|c| **c).count() != 1 {
        return fail("give exactly one of `text`, `url_contains` or `gone`");
    }
    // An element inside a frame is looked for through that frame's session.
    let gone: Option<(i64, Option<FrameRef>)> = match &req.gone {
        Some(r) => {
            let table = state.browser_api.ref_tables.get(&req.block_id);
            match table.and_then(|t| Some((t.refs.get(r).copied()?, t.frames.get(r).cloned()))) {
                Some(g) => Some(g),
                None => return fail(format!("unknown reference {r:?}; take a new snapshot")),
            }
        }
        None => None,
    };
    let timeout = std::time::Duration::from_millis(req.timeout_ms.unwrap_or(10_000).clamp(250, 60_000));
    let started = std::time::Instant::now();
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "wait_for").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let mut worlds = Worlds::new();
    loop {
        let url = page_url(&mut cdp).await;
        let (met, frame) = if let Some(t) = &req.text {
            match find_text(&mut cdp, t, &mut worlds).await {
                Some(frame) => (true, frame),
                None => (false, None),
            }
        } else if let Some(u) = &req.url_contains {
            (url.contains(u.as_str()), None)
        } else if let Some((b, f)) = &gone {
            // A frame that is gone took the element with it.
            let mut on = cdp.in_session(f.as_ref().and_then(|f| f.session.as_deref()));
            (describe_markup(&mut on, *b).await.is_none(), None)
        } else {
            (false, None)
        };
        let waited_ms = started.elapsed().as_millis() as u64;
        if met {
            let _ = cdp.close().await;
            return (StatusCode::OK, Json(ApiResponse::ok(WaitForData { waited_ms, url, frame })));
        }
        if started.elapsed() >= timeout {
            let _ = cdp.close().await;
            return fail(format!(
                "still waiting after {:.1} s (page now {url}); take a snapshot to see where it is",
                timeout.as_secs_f64()
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_is_passed_as_a_string_not_spliced_into_the_script() {
        let text = "Step 2\" ); alert(1); (\"";
        let expression = format!("{HAS_TEXT}({})", serde_json::to_string(text).unwrap());
        assert!(expression.ends_with(r#"("Step 2\" ); alert(1); (\"")"#), "{expression}");
    }

    #[test]
    fn a_match_in_the_top_frame_names_no_frame() {
        let data = WaitForData { waited_ms: 5, url: "http://a.test/".into(), frame: None };
        assert!(serde_json::to_value(&data).unwrap().get("frame").is_none());
        let data = WaitForData { waited_ms: 5, url: "http://a.test/".into(), frame: Some("http://b.test/f".into()) };
        assert_eq!(serde_json::to_value(&data).unwrap()["frame"], "http://b.test/f");
    }
}
