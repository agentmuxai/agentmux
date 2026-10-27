// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The files a Kanban task carries, over HTTP (docs/specs/SPEC_KANBAN_TASK_MEDIA_2026_10_25.md §3, §5):
//!
//!   POST /agentmux/work/media?name=  — one file, streamed to the store and hashed; answers its hash and kind
//!
//! and what `update` and `submit` do with the `media` they carry: attach each file to the task, set the
//! UI-change flag, and, on `submit` of a UI task that waits for the operator, ask for media.

use std::path::PathBuf;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use agentmux_common::time::now_ms;

use crate::backend::storage::work_media::{AttachError, MediaItem, NewMedia};
use crate::backend::storage::work_queue::WorkItem;
use crate::backend::task_media;

use super::work_tasks::clean;
use super::AppState;

/// The longest caption and file name kept.
const MAX_CAPTION: usize = 300;
const MAX_NAME: usize = 200;

/// A file already uploaded, as a task call names it.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct MediaRef {
    pub sha256: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub caption: String,
}

fn error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

#[derive(Deserialize)]
pub(super) struct UploadQuery {
    name: Option<String>,
}

/// `POST /agentmux/work/media?name=`: the body is one file. → 200 `{sha256, name, kind, mime, size}` · 400 empty ·
/// 413 over the size of its kind · 415 not an image or recording.
///
/// Read as a stream and counted as it is written, as the attachments upload is.
pub(super) async fn handle_work_media_upload(State(_state): State<AppState>, Query(q): Query<UploadQuery>, body: Body) -> Response {
    let Some(root) = task_media::default_root() else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "no data directory for task media");
    };
    upload_into(&root, q.name.unwrap_or_default(), body).await
}

pub(super) async fn upload_into(root: &std::path::Path, name: String, body: Body) -> Response {
    let tmp = task_media::new_incoming_path(root);
    if let Some(dir) = tmp.parent() {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
        }
    }
    let written = async {
        let mut out = tokio::fs::File::create(&tmp).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| (StatusCode::BAD_REQUEST, format!("upload interrupted: {e}")))?;
            total += chunk.len() as u64;
            if total > task_media::MAX_RECORDING_BYTES {
                return Err((StatusCode::PAYLOAD_TOO_LARGE, format!("the file is larger than {} MiB", task_media::MAX_RECORDING_BYTES / (1024 * 1024))));
            }
            hasher.update(&chunk);
            out.write_all(&chunk).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        out.flush().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if total == 0 {
            return Err((StatusCode::BAD_REQUEST, "empty body".to_string()));
        }
        Ok((hex::encode(hasher.finalize()), total))
    }
    .await;
    let (sha, size) = match written {
        Ok(v) => v,
        Err((status, msg)) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return error(status, msg);
        }
    };
    let root = root.to_path_buf();
    let tmp2 = tmp.clone();
    let done = tokio::task::spawn_blocking(move || -> Result<Value, (StatusCode, String)> {
        let head = task_media::read_head(&tmp2).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let Some(found) = task_media::sniff(&head) else {
            return Err((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "not a PNG, JPEG, GIF, WebP, SVG, MP4, MOV or WebM file (checked by its content, not its name)".to_string(),
            ));
        };
        if size > task_media::max_bytes(found.kind) {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("a {} is at most {} MiB", found.kind, task_media::max_bytes(found.kind) / (1024 * 1024)),
            ));
        }
        task_media::commit(&root, &tmp2, &sha, &found).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(json!({ "sha256": sha, "kind": found.kind, "mime": found.mime, "size": size }))
    })
    .await;
    match done {
        Ok(Ok(mut v)) => {
            v["name"] = json!(file_name(&name));
            Json(v).into_response()
        }
        Ok(Err((status, msg))) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            error(status, msg)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    }
}

/// A name for the drawer: the last path segment, no longer than [`MAX_NAME`].
fn file_name(given: &str) -> String {
    let base = given.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let base = if base.is_empty() { "file" } else { base };
    clean(base).chars().take(MAX_NAME).collect()
}

/// The stored file for a content hash, and what it is, if it is there.
pub(crate) fn find_stored(root: &std::path::Path, sha: &str) -> Option<(PathBuf, task_media::Sniff)> {
    ["image/png", "image/jpeg", "image/gif", "image/webp", "image/svg+xml", "video/mp4", "video/webm", "video/quicktime"]
        .into_iter()
        .find_map(|mime| {
            let path = task_media::blob_path(root, sha, mime)?;
            let head = task_media::read_head(&path).ok()?;
            let found = task_media::sniff(&head)?;
            (found.mime == mime).then_some((path, found))
        })
}

/// What a task call carries about media.
#[derive(Debug, Default)]
pub(super) struct MediaCall {
    pub media: Vec<MediaRef>,
    pub ui_change: Option<bool>,
    pub no_media_reason: Option<String>,
}

fn bad(text: String) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": text })))
}

/// Attach the files of a call to a task and set its flag. Every file is checked before anything changes, so
/// a bad one fails the call whole. Returns what was attached.
pub(super) fn apply(state: &AppState, item: &WorkItem, agent: &str, uid: &str, call: &MediaCall) -> Result<Vec<MediaItem>, (StatusCode, Json<Value>)> {
    let now = now_ms();
    if call.media.len() > task_media::MAX_FILES_PER_CALL {
        return Err(bad(format!("at most {} files in one call", task_media::MAX_FILES_PER_CALL)));
    }
    let mut ready: Vec<NewMedia> = Vec::new();
    if !call.media.is_empty() {
        let Some(root) = task_media::default_root() else { return Err(bad("no data directory for task media".into())) };
        for m in &call.media {
            let Some((path, found)) = find_stored(&root, &m.sha256) else {
                return Err(bad(format!("{} was not uploaded (or was cleared): upload it again", if m.name.is_empty() { &m.sha256 } else { &m.name })));
            };
            let size = std::fs::metadata(&path).map(|md| md.len() as i64).unwrap_or(0);
            ready.push(NewMedia {
                sha256: m.sha256.clone(),
                name: file_name(&m.name),
                kind: found.kind.to_string(),
                mime: found.mime.to_string(),
                size,
                caption: clean(&m.caption).chars().take(MAX_CAPTION).collect(),
                attached_by: agent.to_string(),
                attached_by_uid: uid.to_string(),
                attempt: item.attempts,
            });
        }
    }
    let mut attached = Vec::new();
    for m in &ready {
        match state.work_store.work_media_add(&item.id, m, now) {
            Ok(Ok(row)) => attached.push(row),
            Ok(Err(why @ (AttachError::TooMany | AttachError::TooBig))) => return Err(bad(why.words())),
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))),
        }
    }
    let reason = call.no_media_reason.as_deref().map(|r| clean(r).chars().take(MAX_CAPTION).collect::<String>());
    if let Err(e) = state.work_store.work_ui_set(&item.id, call.ui_change, reason.as_deref(), now) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))));
    }
    Ok(attached)
}

/// What `submit` asks of a UI task that waits for the operator (§5): media from this attempt, or a reason
/// there is none. `None` lets it through.
pub(super) fn submit_refusal(state: &AppState, id: &str, attempt: i64) -> Option<String> {
    let review = state.work_store.work_board_get(id).ok().flatten().is_some_and(|t| t.card.review_required);
    let (ui, reason) = state.work_store.work_ui_get(id).unwrap_or_default();
    if !(review && ui) || !reason.trim().is_empty() {
        return None;
    }
    let has = state.work_store.work_media_in_attempt(id, attempt).unwrap_or(false);
    (!has).then(|| refusal_text().to_string())
}

pub(super) fn refusal_text() -> &'static str {
    "This task changes the UI and waits for the operator's OK: attach a before/after screenshot, or a short recording for motion or an interaction (`files`), captured from your own test build. If you cannot capture one, say why in `no_media_reason`."
}

/// A reminder for `update` on a UI task with no media yet, when it links a PR (mid-work, so not a refusal).
pub(super) fn pr_reminder(state: &AppState, id: &str, attempt: i64) -> Option<&'static str> {
    let (ui, _) = state.work_store.work_ui_get(id).unwrap_or_default();
    (ui && !state.work_store.work_media_in_attempt(id, attempt).unwrap_or(false))
        .then_some("This task changes the UI: attach screenshots (`files`) from your own test build before you submit it.")
}

/// How the answer lists what was attached.
pub(super) fn attached_json(rows: &[MediaItem]) -> Value {
    Value::Array(rows.iter().map(|r| json!({ "id": r.id, "name": r.name, "kind": r.kind, "size": r.size, "caption": r.caption })).collect())
}

#[cfg(test)]
#[path = "work_media_tests.rs"]
mod tests;
