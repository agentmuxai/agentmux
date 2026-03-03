use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::backend::{docsite, schema};

use super::AppState;

#[derive(serde::Deserialize)]
pub(super) struct FileQueryParams {
    zoneid: Option<String>,
    name: Option<String>,
    #[serde(default)]
    offset: i64,
}

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
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    }
}
