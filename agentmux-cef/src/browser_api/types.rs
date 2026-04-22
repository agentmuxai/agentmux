// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Request/response types for the browser DOM API.

use serde::{Deserialize, Serialize};

// ── Generic response envelope ───────────────────────────────────────────

/// Every `/agentmux/browser/*` response is one of these two shapes.
/// Matches `SPEC_BROWSER_DOM_API.md` §5.1.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ApiResponse<T> {
    Ok { ok: bool, data: T },
    Err { ok: bool, error: String },
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        ApiResponse::Ok { ok: true, data }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        ApiResponse::Err {
            ok: false,
            error: msg.into(),
        }
    }
}

// ── browser.query ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct QueryReq {
    pub block_id: String,
    pub selector: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct QueryData {
    pub matches: Vec<Element>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Element {
    /// Unique CSS path the backend-injected helper computed for this node.
    /// Stable enough to target the same element in a follow-up call,
    /// provided the DOM hasn't been restructured in between.
    pub selector: String,
    pub tag: String,
    /// First ~500 chars of textContent — full text would balloon responses.
    pub text: String,
    pub attrs: serde_json::Map<String, serde_json::Value>,
    pub rect: Rect,
    pub focused: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// ── browser.focus_info ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FocusInfoReq {
    pub block_id: String,
}

#[derive(Debug, Serialize)]
pub struct FocusInfoData {
    /// null when `document.activeElement` is null or the `<body>`
    /// (the default resting state with no focused control).
    pub focused: Option<Element>,
}

// ── browser.eval ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct EvalReq {
    pub block_id: String,
    pub script: String,
    /// If true and the script returns a Promise, wait for it to
    /// resolve before returning. Maps to CDP `Runtime.evaluate`
    /// `awaitPromise`.
    #[serde(default)]
    pub await_promise: bool,
}

#[derive(Debug, Serialize)]
pub struct EvalData {
    /// The serialized JS return value, whatever shape the script
    /// produced. `null` if the script returned `undefined` or threw.
    pub result: serde_json::Value,
    /// CDP's type tag: "object" | "string" | "number" | "boolean" |
    /// "undefined" | "function" | "symbol" | "bigint". Kept so
    /// callers can distinguish `null`-the-value from `null`-the-failure.
    #[serde(rename = "type")]
    pub type_: String,
    /// Populated when the script threw. The message + stack, when
    /// available. `result` is null in this case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception: Option<String>,
}

// ── browser.screenshot ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ScreenshotReq {
    pub block_id: String,
}

#[derive(Debug, Serialize)]
pub struct ScreenshotData {
    /// Base64-encoded PNG bytes — same format CDP's
    /// `Page.captureScreenshot` returns.
    pub png_base64: String,
}

// ── browser.click_element ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ClickElementReq {
    pub block_id: String,
    pub selector: String,
}

// ── browser.focus_element ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct FocusElementReq {
    pub block_id: String,
    pub selector: String,
}

// ── browser.dispatch_key ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DispatchKeyReq {
    pub block_id: String,
    /// Optional CSS selector: focus this element before dispatching
    /// the key event(s). If absent, the key lands on whatever has
    /// focus currently.
    #[serde(default)]
    pub selector: Option<String>,
    /// Send this text as `Input.insertText` (atomic, preserves IME
    /// and autocomplete behaviour). Mutually exclusive with `key`.
    #[serde(default)]
    pub text: Option<String>,
    /// Send this named key as a `keyDown`+`keyUp` pair. Supported:
    /// `Enter`, `Tab`, `Escape`, `Backspace`, `ArrowUp`, `ArrowDown`,
    /// `ArrowLeft`, `ArrowRight`, `Space`. Unknown keys → error.
    #[serde(default)]
    pub key: Option<String>,
}

// ── browser.navigate ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct NavigateReq {
    pub block_id: String,
    pub url: String,
}

// ── browser.back / .forward / .reload ───────────────────────────────────
//
// Share a single request shape — all three only need the target block id.
// These exist so agents driving a browser pane during dev / tests can walk
// its history without a human clicking the toolbar. Also useful for the
// agent workflow where a tool says "open this URL, try to click X, if not
// found go back and try Y."

#[derive(Debug, Deserialize)]
pub struct HistoryReq {
    pub block_id: String,
    /// Reload only: skip the http cache and force a network refetch.
    /// Ignored by `back` / `forward`. Defaults to false.
    #[serde(default)]
    pub ignore_cache: bool,
}

// ── Generic "ok:true" success body for write endpoints ──────────────────

#[derive(Debug, Serialize)]
pub struct AckData {
    pub ok: bool,
}

impl AckData {
    pub fn new() -> Self {
        Self { ok: true }
    }
}
