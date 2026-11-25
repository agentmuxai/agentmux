// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The names of the files a pane's page has in its file inputs, for what a hand-off to the person leaves
//! behind (the agent is told which files the person attached, never where they are or what is in them).
//!
//! Read-only and cheap: in every frame, in a world of its own the page cannot see into, the script walks the
//! document (and the open shadow roots under it) for `<input type=file>` and reads each chosen file's `name`.
//! Nothing else is read: no path (the browser never gives a page one), no size, no type, and never the bytes.
//! What it cannot reach: a closed shadow root, a frame that is not readable (it is counted, not named), an
//! upload widget that empties its input once it has taken the file, and a page that keeps the file somewhere
//! other than an input (a drop zone).

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::act::{fail, unauthorized};
use super::types::ApiResponse;
use crate::state::AppState;

/// Most names kept from one frame, and the longest one: a page may hold many inputs with long names.
const NAMES_PER_FRAME: usize = 50;
const NAME_CHARS: usize = 200;

/// Runs in each frame: `{ names, count }` for its file inputs and their open shadow roots.
const SCRIPT: &str = r#"(() => {
  const names = []; let count = 0, seen = 0;
  const walk = (root) => {
    for (const el of root.querySelectorAll('input[type=file]')) {
      for (const f of Array.from(el.files || [])) { count++; if (names.length < 50) names.push(String(f.name).slice(0, 200)); }
    }
    for (const el of root.querySelectorAll('*')) { if (++seen > 20000) return; if (el.shadowRoot) walk(el.shadowRoot); }
  };
  walk(document);
  return JSON.stringify({ names, count });
})()"#;

#[derive(Debug, Deserialize)]
pub struct FileInputsReq {
    pub block_id: String,
}

/// `POST /agentmux/browser/file_inputs`: `{ names, unreadable }`, the names of the files in the pane's file
/// inputs (every frame), and how many frames could not be read.
pub async fn file_inputs_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<FileInputsReq>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "file_inputs").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let (frames, _) = super::frames::current_frames(&mut cdp).await;
    // No frame listed at all is a read that failed, not a page with no files: say so, so it is not taken for "none".
    if let Err(e) = frames_listed(frames.len()) {
        let _ = cdp.close().await;
        return fail(e);
    }
    let mut names: Vec<String> = Vec::new();
    let mut unreadable = 0usize;
    for f in &frames {
        let mut on = cdp.in_session(f.session.as_deref());
        let read = async {
            let ctx = super::act::isolated_context(&mut on, Some(f.id.as_str())).await?;
            let v = on
                .call("Runtime.evaluate", json!({ "expression": SCRIPT, "contextId": ctx, "returnByValue": true }))
                .await
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(v.pointer("/result/value").and_then(Value::as_str).map(str::to_string))
        };
        match read.await {
            Ok(Some(text)) => names.extend(frame_names(&text)),
            _ => unreadable += 1,
        }
    }
    let _ = cdp.close().await;
    (StatusCode::OK, Json(ApiResponse::ok(json!({ "names": names, "unreadable": unreadable }))))
}

/// `current_frames` answers an empty list when the page's frames could not be listed; a page always has one.
fn frames_listed(count: usize) -> Result<(), String> {
    if count == 0 { Err("couldn't list the page's frames".to_string()) } else { Ok(()) }
}

/// The names one frame's script answered, cut to what the host keeps.
fn frame_names(text: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    v.get("names")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).take(NAMES_PER_FRAME).map(|n| n.chars().take(NAME_CHARS).collect()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frames_answer_is_its_names_cut_to_the_limits() {
        assert_eq!(frame_names(r#"{"names":["a.pdf","b.png"],"count":2}"#), vec!["a.pdf", "b.png"]);
        let long = "x".repeat(500);
        let many: Vec<String> = (0..80).map(|i| format!("f{i}")).collect();
        let text = json!({ "names": many, "count": 80 }).to_string();
        assert_eq!(frame_names(&text).len(), NAMES_PER_FRAME);
        let cut = frame_names(&json!({ "names": [long] }).to_string());
        assert_eq!(cut[0].chars().count(), NAME_CHARS);
    }

    #[test]
    fn a_page_whose_frames_could_not_be_listed_is_an_error_not_an_empty_answer() {
        assert!(frames_listed(0).is_err());
        assert!(frames_listed(1).is_ok());
    }

    #[test]
    fn an_unreadable_answer_names_nothing() {
        assert!(frame_names("not json").is_empty());
        assert!(frame_names("{}").is_empty());
        assert!(frame_names(r#"{"names":[1,null]}"#).is_empty());
    }

    #[test]
    fn the_script_reads_names_only() {
        for word in ["webkitRelativePath", "arrayBuffer", "text()", "FileReader", ".size", ".type", ".path"] {
            assert!(!SCRIPT.contains(word), "the script must not read {word}");
        }
        assert!(SCRIPT.contains("f.name"));
    }
}
