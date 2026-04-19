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
