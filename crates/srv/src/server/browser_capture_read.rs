// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a `Browser` `capture` reads from the page, and the check on what it read: the part of
//! `browser_capture.rs` that asks the host, so the handler keeps to what is stored and told.
//!
//! Three ways to read, all by the host, none of which returns the value to the agent:
//! - the element a reference names (its field value, or the one secret in its text);
//! - text with no reference of its own, found by the label beside it (`near`), which the host gives a
//!   reference of its own so the saved element can be marked like any other;
//! - the page's own copy button (`copy`): the host clicks it with the page's copy intercepted, so the
//!   value never reaches the system clipboard. A click that commits something waits for the person, as an
//!   `act` click does.

use axum::response::Response;
use serde_json::{json, Value};

use super::browser_actions::{note, How, Kind};
use super::browser_capture::{CaptureRequest, DOWNLOAD_WAIT, HOST_WAIT};
use super::browser_capture_shape::{note_miss, Shape};
use super::browser_attention::Waiting;
use super::browser_click_approval::{approve_click, ClickRefused};
use super::ui_handlers::{err_response, proxy_value};
use super::AppState;

/// What the request reads: its reference (empty when `near` names the text instead), and the shape the
/// value must have. An error says what to give.
pub(super) fn what_to_read(req: &CaptureRequest) -> Result<(String, Option<Shape>), String> {
    let ref_ = req.ref_.clone().filter(|r| !r.trim().is_empty());
    let near = req.near.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if near.is_some() && (ref_.is_some() || req.download || req.copy) {
        return Err("near names the text beside a label, instead of a ref: give one or the other, and not with download or copy".into());
    }
    if req.download && req.copy {
        return Err("a ref is a file's download button (download) or a copy button (copy), not both".into());
    }
    let shape = Shape::new(req.prefix.as_deref(), req.pattern.as_deref())?;
    match (ref_, near) {
        (Some(r), None) => Ok((r, shape)),
        (None, Some(_)) => Ok((String::new(), shape)),
        _ => {
            let what = if req.download {
                "the button or link that downloads the file"
            } else if req.copy {
                "the page's copy button"
            } else {
                "the element that shows the secret (or near: the label beside it)"
            };
            Err(format!("capture needs the ref of {what} (from snapshot or find)"))
        }
    }
}

/// Ask the host for the value: `{value, origin, ...}`, and the wait that holds a click the person approved
/// until the host has made it. An error is the answer to give the agent.
pub(super) async fn read_page(state: &AppState, req: &CaptureRequest, block_id: &str, agent: &str, ref_: &str) -> Result<(Value, Option<Waiting>), Response> {
    let route = if req.download {
        "capture_download"
    } else if req.copy {
        "capture_copy"
    } else {
        let body = json!({ "block_id": block_id, "ref": ref_, "near": req.near.as_deref().map(str::trim), "prefix": req.prefix.as_deref().map(str::trim) });
        return proxy_value(state, "capture_value", body, HOST_WAIT).await.map(|v| (v, None)).map_err(|(code, e)| err_response(code, e));
    };
    let body = |approved: Option<&Value>| json!({ "block_id": block_id, "ref": ref_, "timeout_ms": req.timeout_ms, "approved_as": approved });
    let first = proxy_value(state, route, body(None), DOWNLOAD_WAIT).await.map_err(|(code, e)| err_response(code, e))?;
    let Some(ask) = first.get("needs_approval").cloned() else { return Ok((first, None)) };
    let log = note(block_id, agent, Kind::Capture).target(ref_);
    match approve_click(state, block_id, agent, &ask).await {
        Ok((waiting, _)) => proxy_value(state, route, body(Some(&ask)), DOWNLOAD_WAIT).await.map(|v| (v, Some(waiting))).map_err(|(code, e)| err_response(code, e)),
        Err(ClickRefused::NotAsked(code, e)) => Err(err_response(code, e)),
        Err(ClickRefused::Asked(how, code, e)) => {
            log.how(how.unwrap_or(How::Refused(e.clone()))).record(state);
            Err(err_response(code, format!("nothing was clicked or saved: {e}")))
        }
    }
}

/// The value has the shape the agent gave, or nothing is saved. The answer says only that it did not.
pub(super) fn check_shape(shape: Option<&Shape>, value: &str, pane: &str) -> Result<(), String> {
    match shape {
        Some(s) if !s.matches(value) => {
            note_miss(pane);
            Err("nothing was saved: what the page holds there does not match the prefix or pattern you gave (it is not shown to you)".into())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "browser_capture_read_tests.rs"]
mod tests;
