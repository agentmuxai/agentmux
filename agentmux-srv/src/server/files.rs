// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::backend::base::expand_home_dir_safe;
use crate::backend::{docsite, schema};

use super::AppState;

#[derive(serde::Deserialize)]
pub(super) struct FileQueryParams {
    zoneid: Option<String>,
    name: Option<String>,
    #[serde(default)]
    offset: i64,
}

#[derive(serde::Deserialize)]
pub(super) struct LocalFileQueryParams {
    path: Option<String>,
}

// Media pane (SPEC_MEDIA_PANE_2026_07_26.md): local video/image files run
// larger than the 10MB text-editor cap `readeditorfile` uses — this
// session's own generated clips ranged 6-28MB for a few seconds of
// 1920x1080 footage. Sized for local video, not copied from the editor's
// text-oriented number.
const STREAM_LOCAL_FILE_MAX_BYTES: u64 = 500_000_000;

pub(super) async fn handle_wave_file(
    State(state): State<AppState>,
    Query(params): Query<FileQueryParams>,
) -> Response {
    let zone_id = match &params.zoneid {
        Some(z) if !z.is_empty() => z.as_str(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing zoneid"})),
            )
                .into_response()
        }
    };
    let name = match &params.name {
        Some(n) if !n.is_empty() => n.as_str(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing name"})),
            )
                .into_response()
        }
    };

    // Get file metadata
    let file_info = match state.filestore.stat(zone_id, name) {
        Ok(Some(info)) => info,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "file not found"})),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };

    // Read file data
    let (_, data) = match state.filestore.read_at(zone_id, name, params.offset, 0) {
        Ok(result) => result,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };

    // Build X-ZoneFileInfo header (base64-encoded JSON metadata)
    let file_info_json = serde_json::to_string(&file_info).unwrap_or_default();
    let file_info_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &file_info_json);

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        .header("X-ZoneFileInfo", file_info_b64)
        .body(Body::from(data))
        .unwrap_or_else(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to build response",
            )
                .into_response()
        })
}

pub(super) async fn handle_schema(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    let app_path = if state.app_path.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "app path not configured"})),
        )
            .into_response();
    } else {
        PathBuf::from(&state.app_path)
    };

    let schema_dir = schema::get_schema_dir(&app_path);
    let name = match schema::normalize_schema_request(&path) {
        Some(n) => n,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid schema path"})),
            )
                .into_response()
        }
    };

    match schema::resolve_schema_path(&schema_dir, &name) {
        Some(file_path) => match std::fs::read(&file_path) {
            Ok(data) => Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", schema::SCHEMA_CONTENT_TYPE)
                .body(Body::from(data))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(super) async fn handle_docsite(AxumPath(path): AxumPath<String>) -> Response {
    match docsite::resolve_docsite_path(&path) {
        Some(file_path) => {
            let content_type = mime_from_path(&file_path);
            match std::fs::read(&file_path) {
                Ok(data) => Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", content_type)
                    .body(Body::from(data))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn mime_from_path(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("webm") => "video/webm",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    }
}

/// Media pane (SPEC_MEDIA_PANE_2026_07_26.md): serve an arbitrary local file
/// by absolute path, for `<img>`/`<video>` display. Deliberately matches
/// `readeditorfile`'s existing posture (any absolute path the frontend
/// sends, gated by OS-level permissions rather than an in-app allowlist —
/// see `editor_handlers.rs:47-53`'s "root scoping, not a sandbox" note) so
/// this route isn't a stricter one-off next to an already-shipped read path
/// with the same shape. Size-capped (see `STREAM_LOCAL_FILE_MAX_BYTES`)
/// rather than truly unbounded.
pub(super) async fn handle_stream_local_file(
    Query(params): Query<LocalFileQueryParams>,
) -> Response {
    let raw_path = match &params.path {
        Some(p) if !p.is_empty() => p.as_str(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing path"})),
            )
                .into_response()
        }
    };

    let expanded = expand_home_dir_safe(raw_path);
    let path = expanded.as_path();

    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("stream-local-file: {e}")})),
            )
                .into_response()
        }
    };
    if !metadata.is_file() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "stream-local-file: not a file"})),
        )
            .into_response();
    }
    if metadata.len() > STREAM_LOCAL_FILE_MAX_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error": "stream-local-file: file too large (>500MB)"})),
        )
            .into_response();
    }

    match std::fs::read(path) {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", mime_from_path(path))
            .body(Body::from(data))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("stream-local-file: {e}")})),
        )
            .into_response(),
    }
}
