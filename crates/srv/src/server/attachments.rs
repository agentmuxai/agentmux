// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP side of image attachments
//! (docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.2, §6.5):
//!
//! - `GET  /api/v1/attachments/:id/:kind` — serve `original`, `thumb` or `send`.
//! - `POST /api/v1/attachments/upload?name=` — the paste fallback: the raw
//!   body is one image, streamed to disk and hashed as it arrives.
//!
//! Both sit in `authed_routes`, so they need `X-AuthKey`.

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use futures_util::StreamExt;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::AppState;
use crate::backend::attachments::{self, Kind};

fn service(state: &AppState) -> std::sync::Arc<attachments::Service> {
    attachments::init(state.broker.clone(), state.config_watcher.clone())
}

fn error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

/// `GET /api/v1/attachments/:id/:kind`. The original is immutable by hash;
/// thumb and send-copy change with the transform fingerprint, so everything
/// is `no-cache` with an `ETag` of `<id>-<fp>` and a 304 on a match.
pub(super) async fn handle_attachment_file(
    State(state): State<AppState>,
    Path((id, kind)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !attachments::is_valid_id(&id) {
        return error(StatusCode::BAD_REQUEST, "invalid attachment id");
    }
    let Some(kind) = Kind::parse(&kind) else {
        return error(
            StatusCode::BAD_REQUEST,
            "kind must be original, thumb or send",
        );
    };
    let svc = service(&state);
    let edge = svc.limits().send_max_edge;
    let fp = attachments::store::fingerprint(edge);
    let found = if kind == Kind::Original {
        // The original is served straight from blobs/: it must not depend on
        // the derive pipeline succeeding.
        svc.store().original(&id)
    } else {
        // Re-derives (under the service's limits) when the fingerprint
        // changed since the attachment was processed.
        match svc.ensure_derived(&id).await {
            Some(_) => svc.store().file(&id, &fp, kind),
            None => None,
        }
    };
    let Some((path, mime)) = found else {
        return error(StatusCode::NOT_FOUND, "attachment not found");
    };
    // The original never changes for an id; derived files change with the
    // fingerprint.
    let etag = if kind == Kind::Original {
        format!("\"{id}\"")
    } else {
        format!("\"{id}-{}\"", attachments::store::fingerprint(edge))
    };
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == etag)
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return error(StatusCode::NOT_FOUND, "attachment not found"),
    };
    let len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, len.to_string())
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::ETAG, etag)
        .body(Body::from_stream(tokio_util::io::ReaderStream::new(file)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[derive(serde::Deserialize)]
pub(super) struct UploadQuery {
    /// Display name for the returned info ("Pasted image …png").
    name: Option<String>,
}

/// `POST /api/v1/attachments/upload?name=` — body is one image file.
/// → 200 `AttachmentInfo` · 400 empty · 413 over the per-file cap ·
///   415 not a supported image · 500 I/O.
///
/// The body is read as a stream and counted as it is written: axum's
/// `DefaultBodyLimit` only governs buffering extractors, so it would not
/// stop a streamed body.
pub(super) async fn handle_attachment_upload(
    State(state): State<AppState>,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> Response {
    let svc = service(&state);
    let cap = svc.limits().max_total_bytes;
    let name = q
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Pasted image".to_string());

    let tmp = svc.store().new_incoming_path();
    let written = async {
        let mut out = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| (StatusCode::BAD_REQUEST, format!("upload interrupted: {e}")))?;
            total += chunk.len() as u64;
            if total > cap {
                return Err((
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!(
                        "The image is larger than the {} MB limit.",
                        cap / (1024 * 1024)
                    ),
                ));
            }
            hasher.update(&chunk);
            out.write_all(&chunk)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        out.flush()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        drop(out);
        if total == 0 {
            return Err((StatusCode::BAD_REQUEST, "empty body".to_string()));
        }
        Ok(hex::encode(hasher.finalize()))
    }
    .await;

    let id = match written {
        Ok(id) => id,
        Err((status, msg)) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return error(status, msg);
        }
    };

    match svc.commit_upload(tmp, id, name).await {
        Ok(info) => Json(info).into_response(),
        Err(e) => {
            let status = match e.code {
                "unsupported" | "heic" => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "too_large_dimensions" => StatusCode::PAYLOAD_TOO_LARGE,
                "io" => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::UNPROCESSABLE_ENTITY,
            };
            (status, Json(json!({ "error": e.message, "code": e.code }))).into_response()
        }
    }
}
