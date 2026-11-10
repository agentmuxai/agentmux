// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `set_files` with `chooser: true`: the files go into the file chooser the page opened and the host
//! kept for the agent (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §6,
//! `client/chooser_hold.rs`). Whether the agent may at all, and which files, is srv's decision (the
//! grant's uploads power and the workspace rules); this route re-checks that the page is still on the
//! granted sites, and that the chooser is still waiting and still the page's own. The browser does not
//! say which frame opened a chooser, so a page with a frame from a site the grant does not cover gets
//! none: the files could go to that frame's input.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::act::{fail, page_url, unauthorized, UploadFile};
use super::frames::{current_frames, RawFrame};
use super::types::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ChooserFilesReq {
    pub block_id: String,
    pub files: Vec<UploadFile>,
    /// The sites the person's grant is bound to; the page must still be on one.
    pub allowed_origins: Vec<String>,
}

/// `POST /agentmux/browser/chooser_files` — answers `{ after: [{ name, size }] }`.
pub async fn chooser_files_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ChooserFilesReq>,
) -> (axum::http::StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "chooser_files").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let now = page_url(&mut cdp).await;
    let (frames, _) = current_frames(&mut cdp).await;
    let _ = cdp.close().await;
    if !agentmux_common::allowed_origins::allows(&req.allowed_origins, &now) {
        return fail("no file chooser was filled: the page is off the sites the person granted");
    }
    if let Some(why) = refuse_frames(&frames, &req.allowed_origins) {
        return fail(why);
    }
    // The host's data folder is the person's own: nothing another local user can reach.
    let Some(data) = state.version_data_dir.lock().clone().filter(|d| !d.is_empty()) else {
        return fail("no file chooser was filled: the host has no data folder to keep the chosen files in");
    };
    let base = std::path::Path::new(&data).join("chooser-files");
    match crate::client::chooser_hold::fill(&req.block_id, &now, &req.files, &base) {
        Ok(after) => (axum::http::StatusCode::OK, Json(ApiResponse::ok(json!({ "after": after })))),
        Err(e) => fail(e),
    }
}

/// Why no chooser is filled for a page with these frames, or `None`: the browser doesn't say which frame
/// opened it, so every frame showing a web site must be on the sites the person granted. A frame
/// that is `about:blank` or built from markup takes its parent's site and is judged through it.
fn refuse_frames(frames: &[RawFrame], allowed: &[String]) -> Option<String> {
    if frames.is_empty() {
        return Some("no file chooser was filled: the page's frames couldn't be read".to_string());
    }
    let off = frames
        .iter()
        .filter(|f| f.parent_id.is_some())
        .find(|f| agentmux_common::allowed_origins::target_origin(&f.url).is_some() && !agentmux_common::allowed_origins::allows(allowed, &f.url))?;
    let site = agentmux_common::allowed_origins::target_origin(&off.url).unwrap_or_else(|| off.url.clone());
    Some(format!(
        "no file chooser was filled: the page has a frame from {site}, which the person's grant doesn't cover, and the browser doesn't say which frame opened the chooser, so the files could go to that frame. Ask the user to add {site} to the grant (Settings, Agents on sites), or use set_files with the ref of the control inside the frame"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: &str, parent: Option<&str>, url: &str) -> RawFrame {
        RawFrame { id: id.into(), parent_id: parent.map(str::to_string), url: url.into(), origin: String::new(), session: None }
    }

    fn granted(sites: &[&str]) -> Vec<String> {
        sites.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_page_alone_or_with_frames_on_granted_sites_gets_the_files() {
        let a = granted(&["https://a.example", "https://shell.example"]);
        assert_eq!(refuse_frames(&[frame("1", None, "https://a.example/")], &a), None);
        let with = [frame("1", None, "https://a.example/"), frame("2", Some("1"), "https://shell.example/term"), frame("3", Some("1"), "about:blank")];
        assert_eq!(refuse_frames(&with, &a), None, "an about:blank frame has no site of its own");
    }

    #[test]
    fn a_frame_from_a_site_the_grant_does_not_cover_refuses_the_chooser_and_names_the_site() {
        let a = granted(&["https://a.example"]);
        let frames = [frame("1", None, "https://a.example/"), frame("2", Some("1"), "https://ads.example/x")];
        let why = refuse_frames(&frames, &a).unwrap();
        assert!(why.contains("https://ads.example") && why.contains("Agents on sites"), "{why}");
    }

    #[test]
    fn frames_that_could_not_be_read_refuse_rather_than_guess() {
        assert!(refuse_frames(&[], &granted(&["https://a.example"])).is_some());
    }
}
