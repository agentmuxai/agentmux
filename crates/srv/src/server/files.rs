// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

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

pub(super) async fn handle_mux_file(
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
        Some("wav") => "audio/wav",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    }
}

/// Media pane (SPEC_MEDIA_PANE_2026_07_26.md): serve an arbitrary local file
/// by absolute path, for `<img>`/`<video>` display. Deliberately matches
/// `readeditorfile`'s existing posture (any absolute path the frontend
/// sends, gated by OS-level permissions rather than an in-app allowlist —
/// see the "root scoping, not a sandbox" note on the macOS `list_drives` in
/// `editor_handlers.rs`) so
/// this route isn't a stricter one-off next to an already-shipped read path
/// with the same shape. Size-capped (see `STREAM_LOCAL_FILE_MAX_BYTES`)
/// rather than truly unbounded.
///
/// Honours a single `Range` (`206 Partial Content`), so a `<video>` poster can
/// read just the start of a file.
pub(super) async fn handle_stream_local_file(
    Query(params): Query<LocalFileQueryParams>,
    headers: HeaderMap,
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

    // Async metadata + streamed read, not std::fs::read — the earlier
    // synchronous-read version blocked a shared Tokio worker thread and
    // materialized the whole file in memory for every request, which is a
    // real problem at this route's size ceiling (up to 500MB) and given the
    // Media pane's live-update re-fetch pattern (repeated large reads as
    // watched files change). Flagged in review (ReAgent P1, Codex P2).
    let metadata = match tokio::fs::metadata(path).await {
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

    let total = metadata.len();
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map_or(ByteRange::Full, |h| parse_byte_range(h, total));
    if range == ByteRange::Unsatisfiable {
        return Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{total}"))
            .header(header::ACCEPT_RANGES, "bytes")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("stream-local-file: {e}")})),
            )
                .into_response()
        }
    };
    let builder = Response::builder()
        .header(header::CONTENT_TYPE, mime_from_path(path))
        .header(header::ACCEPT_RANGES, "bytes");
    let response = match range {
        // Only the requested bytes: seek, then cap the streamed read. Lets a
        // <video> poster read a file's first bytes without fetching all of it
        // (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §5).
        ByteRange::Partial(start, end) => {
            if let Err(e) = file.seek(std::io::SeekFrom::Start(start)).await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("stream-local-file: {e}")})),
                )
                    .into_response();
            }
            let len = end - start + 1;
            builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
                .header(header::CONTENT_LENGTH, len.to_string())
                .body(Body::from_stream(tokio_util::io::ReaderStream::new(file.take(len))))
        }
        _ => builder
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, total.to_string())
            .body(Body::from_stream(tokio_util::io::ReaderStream::new(file))),
    };
    response.unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// A request's `Range`, resolved against the file's size.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ByteRange {
    /// No usable range: serve the whole file (`200`).
    Full,
    /// Inclusive byte offsets, both within the file (`206`).
    Partial(u64, u64),
    /// A valid range that starts past the end (`416`).
    Unsatisfiable,
}

/// Parses a single `bytes=` range (`a-b`, `a-`, or the suffix form `-n`).
/// Anything this doesn't understand — another unit, several ranges, `b < a`,
/// junk — serves the whole file, which RFC 9110 §14.2 allows a server to do
/// for any `Range` it chooses to ignore.
pub(super) fn parse_byte_range(header: &str, total: u64) -> ByteRange {
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return ByteRange::Full;
    };
    if spec.contains(',') {
        return ByteRange::Full;
    }
    let Some((a, b)) = spec.split_once('-') else {
        return ByteRange::Full;
    };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        let Ok(n) = b.parse::<u64>() else {
            return ByteRange::Full;
        };
        if n == 0 || total == 0 {
            return ByteRange::Unsatisfiable;
        }
        return ByteRange::Partial(total.saturating_sub(n), total - 1);
    }
    let Ok(start) = a.parse::<u64>() else {
        return ByteRange::Full;
    };
    let end = if b.is_empty() {
        None
    } else {
        match b.parse::<u64>() {
            Ok(e) if e >= start => Some(e),
            _ => return ByteRange::Full,
        }
    };
    if start >= total {
        return ByteRange::Unsatisfiable;
    }
    ByteRange::Partial(start, end.map_or(total - 1, |e| e.min(total - 1)))
}

#[cfg(test)]
mod stream_local_file_range_tests {
    use super::*;
    use axum::http::{header, HeaderMap, HeaderValue};

    // ── parse_byte_range ────────────────────────────────────────────────

    #[test]
    fn a_closed_range_is_partial() {
        assert_eq!(parse_byte_range("bytes=0-99", 1000), ByteRange::Partial(0, 99));
        assert_eq!(parse_byte_range("bytes=100-199", 1000), ByteRange::Partial(100, 199));
    }

    #[test]
    fn an_end_past_the_file_is_clamped() {
        assert_eq!(parse_byte_range("bytes=900-5000", 1000), ByteRange::Partial(900, 999));
    }

    #[test]
    fn an_open_range_runs_to_the_end() {
        assert_eq!(parse_byte_range("bytes=500-", 1000), ByteRange::Partial(500, 999));
    }

    #[test]
    fn a_suffix_range_is_the_last_n_bytes() {
        assert_eq!(parse_byte_range("bytes=-100", 1000), ByteRange::Partial(900, 999));
        assert_eq!(parse_byte_range("bytes=-5000", 1000), ByteRange::Partial(0, 999));
    }

    #[test]
    fn a_start_past_the_end_is_unsatisfiable() {
        assert_eq!(parse_byte_range("bytes=1000-", 1000), ByteRange::Unsatisfiable);
        assert_eq!(parse_byte_range("bytes=0-0", 0), ByteRange::Unsatisfiable);
        assert_eq!(parse_byte_range("bytes=-0", 1000), ByteRange::Unsatisfiable);
    }

    #[test]
    fn anything_else_serves_the_whole_file() {
        for h in ["bytes=5-2", "bytes=abc-", "items=0-1", "bytes=0-1,5-9", "bytes=-", ""] {
            assert_eq!(parse_byte_range(h, 1000), ByteRange::Full, "{h:?}");
        }
    }

    // ── handler ─────────────────────────────────────────────────────────

    fn file_with(bytes: &[u8]) -> tempfile::NamedTempFile {
        let f = tempfile::Builder::new().suffix(".webm").tempfile().unwrap();
        std::fs::write(f.path(), bytes).unwrap();
        f
    }

    async fn get(path: &std::path::Path, range: Option<&str>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut headers = HeaderMap::new();
        if let Some(r) = range {
            headers.insert(header::RANGE, HeaderValue::from_str(r).unwrap());
        }
        let params = LocalFileQueryParams { path: Some(path.to_string_lossy().into_owned()) };
        let resp = handle_stream_local_file(Query(params), headers).await;
        let status = resp.status();
        let hdrs = resp.headers().clone();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec();
        (status, hdrs, body)
    }

    #[tokio::test]
    async fn no_range_serves_the_whole_file_and_advertises_ranges() {
        let f = file_with(b"0123456789");
        let (status, h, body) = get(f.path(), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"0123456789");
        assert_eq!(h[header::ACCEPT_RANGES], "bytes");
        assert_eq!(h[header::CONTENT_LENGTH], "10");
    }

    #[tokio::test]
    async fn a_range_returns_206_with_just_those_bytes() {
        let f = file_with(b"0123456789");
        let (status, h, body) = get(f.path(), Some("bytes=2-5")).await;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(body, b"2345");
        assert_eq!(h[header::CONTENT_RANGE], "bytes 2-5/10");
        assert_eq!(h[header::CONTENT_LENGTH], "4");
        assert_eq!(h[header::CONTENT_TYPE], "video/webm");
    }

    #[tokio::test]
    async fn an_unsatisfiable_range_is_416_with_the_size() {
        let f = file_with(b"0123456789");
        let (status, h, _) = get(f.path(), Some("bytes=50-")).await;
        assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(h[header::CONTENT_RANGE], "bytes */10");
    }
}
