// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `/agentmux/browser/capture_download`, for srv only: a file a page offers once
//! (a GitHub App's private key) goes into the Vault without touching the disk
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §15.1).
//!
//! The click is checked like `act`'s: a file input is refused, and a click that
//! commits something (GitHub's "Generate a private key" submits a form) answers
//! `needs_approval` without clicking; srv asks the person in the pane and calls
//! again with what they approved, which the page must still show.
//!
//! It clicks the element a reference names, with the pane's page responses held
//! for a moment each (CDP `Fetch` at the response stage, documents only). The
//! first one that is a download is read into memory and aborted, so CEF never
//! sees a download; every other response goes on. Meanwhile the pane's download
//! handler refuses any download that reaches it (a file the page built itself, or
//! one from a frame in another process), which never goes to disk either: the
//! capture then says so. Every exit turns `Fetch` off again, or the pane's pages
//! would wait on it.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::act::{check_click, click_element, fail, isolated_context, resolve, unauthorized, ClickCheck};
use super::cdp::{subscribe, CdpSession};
use super::types::ApiResponse;
use super::vault_fill::locate;
use crate::client::downloads::{hold_for_vault, safe_download_name};
use crate::state::AppState;

/// The largest file a capture takes: a key or a certificate, not a document.
pub const MAX_FILE: usize = 64 * 1024;
/// How long it waits for the download after the click, unless told otherwise.
const DEFAULT_WAIT: Duration = Duration::from_secs(60);
/// The longest it waits (spec §15.1: within 2 minutes).
pub const MAX_WAIT: Duration = Duration::from_secs(120);

#[derive(Debug, Deserialize)]
pub struct DownloadReq {
    pub block_id: String,
    /// The button or link that downloads the file.
    #[serde(rename = "ref")]
    pub ref_: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// The `needs_approval` the person approved, sent back with the approved click.
    #[serde(default)]
    pub approved_as: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct Downloaded {
    /// The file's text.
    pub value: String,
    /// The origin of the frame the element is in.
    pub origin: String,
    pub page: String,
    /// The file's name, made safe, as the page named it.
    pub name: String,
}

/// `POST /agentmux/browser/capture_download`.
pub async fn capture_download_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<DownloadReq>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !super::routes::authorized_pub(&headers, &state.ipc_token) {
        return unauthorized();
    }
    let mut cdp = match super::routes::open_dedicated_pane(&state, &req.block_id, "vault-capture-download").await {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let result = download(&state, &mut cdp, &req).await;
    let _ = cdp.close().await;
    match result {
        Ok(data) => (StatusCode::OK, Json(ApiResponse::ok(data))),
        Err(e) => fail(e),
    }
}

/// Turns `Fetch` off when dropped, whatever ended the capture (an error, or srv
/// giving up on the request).
struct FetchOff(Option<CdpSession>);

impl FetchOff {
    async fn now(mut self) {
        if let Some(mut s) = self.0.take() {
            let _ = s.call("Fetch.disable", json!({})).await;
        }
    }
}

impl Drop for FetchOff {
    fn drop(&mut self) {
        if let Some(mut s) = self.0.take() {
            tokio::spawn(async move {
                let _ = s.call("Fetch.disable", json!({})).await;
            });
        }
    }
}

/// `{"needs_approval": ...}` when the click waits for the person, else [`Downloaded`].
async fn download(state: &Arc<AppState>, root: &mut CdpSession, req: &DownloadReq) -> Result<Value, String> {
    let (located, backend, frame) = locate(state, &req.block_id, &req.ref_, root).await?;
    let browser_id = root.browser_id().ok_or("the pane closed")?;
    let mut in_frame = root.in_session(frame.as_ref().and_then(|f| f.session.as_deref()));
    let ctx = isolated_context(&mut in_frame, frame.as_ref().map(|f| f.frame_id.as_str())).await?;
    let obj = resolve(&mut in_frame, backend, ctx, &req.ref_).await?;
    let in_a_frame = frame.as_ref().map(|f| format!(" (in a frame from {})", f.origin)).unwrap_or_default();
    // Checked before anything is held: a click waiting for approval holds nothing.
    let approved = req.approved_as.is_some();
    if let ClickCheck::Ask(ask) = check_click(&mut in_frame, &obj, &located.facts, &in_a_frame, &located.page, req.approved_as.as_ref(), approved).await? {
        return Ok(json!({ "needs_approval": ask }));
    }

    let hold = hold_for_vault(&req.block_id).ok_or("this pane is already waiting for a download for the Vault")?;
    let mut paused = subscribe(browser_id, "Fetch.requestPaused");
    root.call(
        "Fetch.enable",
        json!({ "patterns": [{ "urlPattern": "*", "resourceType": "Document", "requestStage": "Response" }] }),
    )
    .await?;
    let fetch = FetchOff(Some(root.in_session(None)));
    click_element(&mut in_frame, root, frame.as_ref(), &obj).await?;

    let wait = req.timeout_ms.map(Duration::from_millis).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    let deadline = tokio::time::Instant::now() + wait;
    let file = loop {
        if hold.refused() > 0 {
            break Err("the page made the file itself, or in a frame AgentMux can't take it from, so it couldn't go to the Vault: it wasn't saved anywhere".to_string());
        }
        let tick = (tokio::time::Instant::now() + Duration::from_millis(250)).min(deadline);
        let event = match tokio::time::timeout_at(tick, paused.rx.recv()).await {
            Ok(Some(e)) => e,
            Ok(None) => break Err("the pane closed before the file came".to_string()),
            Err(_) if tick >= deadline => {
                break Err(format!("no download came within {} seconds of clicking {}", wait.as_secs(), req.ref_));
            }
            Err(_) => continue,
        };
        // Fetch is on for the page's own session only.
        if event.get("sessionId").is_some() {
            continue;
        }
        let params = event.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = params.get("requestId").and_then(Value::as_str).map(str::to_string) else { continue };
        if !is_download(&params) {
            let _ = root.call("Fetch.continueRequest", json!({ "requestId": id })).await;
            continue;
        }
        let body = root.call("Fetch.getResponseBody", json!({ "requestId": id })).await;
        // Aborted, so it never becomes a download.
        let _ = root.call("Fetch.failRequest", json!({ "requestId": id, "errorReason": "Aborted" })).await;
        break body.and_then(|b| file_text(&b, &req.ref_)).map(|text| (text, file_name(&params)));
    };
    // Anything else held meanwhile goes on before Fetch is turned off.
    while let Ok(event) = paused.rx.try_recv() {
        if let Some(id) = event.pointer("/params/requestId").and_then(Value::as_str).filter(|_| event.get("sessionId").is_none()) {
            let _ = root.call("Fetch.continueRequest", json!({ "requestId": id })).await;
        }
    }
    fetch.now().await;
    drop(paused);
    drop(hold);
    let (value, name) = file?;
    serde_json::to_value(Downloaded { value, origin: located.origin, page: located.page, name }).map_err(|e| e.to_string())
}

fn header<'a>(params: &'a Value, name: &str) -> Option<&'a str> {
    params
        .get("responseHeaders")?
        .as_array()?
        .iter()
        .find(|h| h.get("name").and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case(name)))?
        .get("value")?
        .as_str()
}

/// Types a browser saves rather than shows: a key, a certificate, bytes.
const DOWNLOAD_TYPES: &[&str] = &[
    "application/octet-stream",
    "application/x-pem-file",
    "application/pkcs8",
    "application/pkix-cert",
    "application/x-x509-ca-cert",
    "application/x-pkcs12",
    "application/pkcs12",
];

/// Whether a paused response is a download: a successful one the server says to
/// save (`Content-Disposition: attachment`), or of a type a browser saves.
fn is_download(params: &Value) -> bool {
    let status = params.get("responseStatusCode").and_then(Value::as_i64).unwrap_or(0);
    if !(200..300).contains(&status) {
        return false;
    }
    let attachment = header(params, "content-disposition").is_some_and(|d| d.trim().to_ascii_lowercase().starts_with("attachment"));
    let mime = header(params, "content-type").map(|t| t.split(';').next().unwrap_or("").trim().to_ascii_lowercase()).unwrap_or_default();
    attachment || DOWNLOAD_TYPES.contains(&mime.as_str())
}

/// The file's name: `filename*` or `filename` in its `Content-Disposition`, else
/// the URL's last part, made safe like any download's.
fn file_name(params: &Value) -> String {
    let disposition = header(params, "content-disposition").unwrap_or("");
    let from_header = disposition.split(';').map(str::trim).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim().trim_matches('"');
        match k.as_str() {
            "filename*" => Some(v.rsplit("''").next().unwrap_or(v).to_string()),
            "filename" => Some(v.to_string()),
            _ => None,
        }
    });
    let url = params.pointer("/request/url").and_then(Value::as_str).unwrap_or("");
    let from_url = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("");
    safe_download_name(from_header.as_deref().unwrap_or(from_url))
}

/// The file's text from `Fetch.getResponseBody`: text only (a key or a
/// certificate in PEM), at most [`MAX_FILE`]. The refusals never quote it.
fn file_text(body: &Value, ref_: &str) -> Result<String, String> {
    let raw = body.get("body").and_then(Value::as_str).unwrap_or("");
    let bytes = if body.get("base64Encoded").and_then(Value::as_bool).unwrap_or(false) {
        base64::engine::general_purpose::STANDARD.decode(raw).map_err(|_| "the file couldn't be read".to_string())?
    } else {
        raw.as_bytes().to_vec()
    };
    if bytes.is_empty() {
        return Err(format!("the file {ref_} downloads is empty: there is nothing to save"));
    }
    if bytes.len() > MAX_FILE {
        return Err(format!("the file {ref_} downloads is over {} KB, more than a key or certificate: it wasn't saved anywhere", MAX_FILE / 1024));
    }
    String::from_utf8(bytes).map_err(|_| "the file isn't text (a key or certificate in PEM is): binary files can't go into the Vault yet, and it wasn't saved anywhere".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paused(status: i64, headers: &[(&str, &str)], url: &str) -> Value {
        let h: Vec<Value> = headers.iter().map(|(n, v)| json!({ "name": n, "value": v })).collect();
        json!({ "requestId": "r1", "request": { "url": url }, "responseStatusCode": status, "responseHeaders": h })
    }

    #[test]
    fn a_download_is_an_attachment_or_a_type_a_browser_saves() {
        let pem = paused(200, &[("Content-Disposition", "attachment; filename=\"my-app.2026-10-30.private-key.pem\"")], "https://github.com/settings/apps/x/keys");
        assert!(is_download(&pem));
        assert!(is_download(&paused(200, &[("content-type", "application/x-pem-file")], "https://x.test/k")));
        assert!(!is_download(&paused(200, &[("Content-Type", "text/html; charset=utf-8")], "https://github.com/settings")), "a page goes on");
        assert!(!is_download(&paused(302, &[("Content-Disposition", "attachment")], "https://x.test/k")), "a redirect goes on");
    }

    #[test]
    fn its_name_comes_from_the_header_or_the_url_and_is_made_safe() {
        let named = paused(200, &[("Content-Disposition", "attachment; filename=\"..\\\\key.pem\"")], "https://x.test/download");
        assert_eq!(file_name(&named), "key.pem");
        let encoded = paused(200, &[("Content-Disposition", "attachment; filename*=UTF-8''app-key.pem")], "https://x.test/d");
        assert_eq!(file_name(&encoded), "app-key.pem");
        assert_eq!(file_name(&paused(200, &[], "https://x.test/keys/app.pem?token=1")), "app.pem");
    }

    #[test]
    fn only_a_small_text_file_is_taken_and_a_refusal_never_quotes_it() {
        let text = "recovery codes: abcd-1234 efgh-5678";
        let encoded = base64::engine::general_purpose::STANDARD.encode(text);
        assert_eq!(file_text(&json!({ "body": encoded, "base64Encoded": true }), "e4").unwrap(), text);
        assert_eq!(file_text(&json!({ "body": text, "base64Encoded": false }), "e4").unwrap(), text);
        assert!(file_text(&json!({ "body": "" }), "e4").unwrap_err().contains("empty"));
        let big = "a".repeat(MAX_FILE + 1);
        let err = file_text(&json!({ "body": big }), "e4").unwrap_err();
        assert!(err.contains("64 KB") && !err.contains("aaaa"), "{err}");
        let binary = base64::engine::general_purpose::STANDARD.encode([0xff, 0xfe, 0x00, 0x81]);
        assert!(file_text(&json!({ "body": binary, "base64Encoded": true }), "e4").unwrap_err().contains("isn't text"));
    }
}
