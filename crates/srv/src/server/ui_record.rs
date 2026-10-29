// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `UI` action `record`: a short animated GIF of the caller's own pane, for a task that changes the UI
//! (docs/specs/SPEC_KANBAN_TASK_MEDIA_2026_10_25.md §6, phase 2). `start` begins collecting frames; `stop`
//! (or the time limit) finishes the file and returns its path, which goes straight into `files` of
//! `Tasks` update/submit.
//!
//! A frame is what `screenshot` returns: the host's capture of the caller's own pane (or a browser pane
//! it opened), clipped to that pane, with the secret fields and the Vault the host found just before and
//! just after the capture blacked out here (`screenshot_redact`). So the target rules and the redaction
//! are the screenshot's own, frame by frame. A frame the host could not check for secret fields is left
//! out and counted, never kept. Frames are encoded as they arrive, so memory stays at about the size of
//! the file, and the file is kept under the size one attached image may have.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use agentmux_common::api_types::{UiRecordRequest, UiRecordResponse};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine as _;
use image::codecs::gif::{GifEncoder, Repeat};
use image::imageops::FilterType;
use image::{Delay, Frame, RgbaImage};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::ui_handlers::{err_response, get_host_ipc, proxy_to_host_timeout, target_block_id, verified_block_id};
use super::AppState;

/// The longest recording, in seconds (the spec's limit).
pub(crate) const MAX_SECONDS: u32 = 30;
/// The most frames a second (the spec's limit), and what a call gets without asking.
pub(crate) const MAX_FPS: u32 = 10;
pub(crate) const DEFAULT_FPS: u32 = 5;
/// Frames are scaled down to this width: a recording is for looking at a change, not for reading text.
pub(crate) const FRAME_WIDTH: u32 = 800;
/// The file stays under the size one attached image may have (task media: `MAX_IMAGE_BYTES`; a GIF is an image).
pub(crate) const MAX_BYTES: usize = crate::backend::task_media::MAX_IMAGE_BYTES as usize - 1024 * 1024;
/// The shortest a frame stays on screen: GIF counts in hundredths of a second and viewers treat less as slow.
const MIN_DELAY_MS: u64 = 20;
/// How long a host capture may take before the frame is given up on. A capture takes about 70 ms; a pane in a
/// minimized or hidden window never answers.
const FRAME_TIMEOUT: Duration = Duration::from_secs(3);
/// How many frames in a row may fail before the recording ends with what it has.
const MAX_FAILED_IN_A_ROW: u32 = 5;
/// Recordings in the temp folder older than this are removed when a new one is written.
const RETENTION: Duration = Duration::from_secs(60 * 60);

/// What a finished recording is.
#[derive(Debug, Clone)]
pub(crate) struct Finished {
    pub path: std::path::PathBuf,
    pub frames: u32,
    /// Frames left out: the host could not check the page for secret fields, or a capture failed.
    pub skipped: u32,
    pub seconds: f32,
    /// Why it ended: `stopped`, `time limit`, `size limit` or `capture failed`.
    pub ended: &'static str,
}

/// A GIF written one frame at a time. A frame's delay is the time until the next one, so each is held back
/// until its successor arrives (or the end), and then encoded.
pub(crate) struct GifBuilder {
    encoder: Option<GifEncoder<SharedBuf>>,
    buf: SharedBuf,
    held: Option<(RgbaImage, Instant)>,
    frames: u32,
    last_delay_ms: u64,
    /// The first frame's size: every frame of a GIF has it, so a pane resized meanwhile is stretched to it.
    size: Option<(u32, u32)>,
}

/// A growable buffer the encoder writes into and the builder can measure and take back.
#[derive(Clone, Default)]
pub(crate) struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl SharedBuf {
    fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl GifBuilder {
    pub(crate) fn new() -> Self {
        let buf = SharedBuf::default();
        // Speed 20: quantizing each frame is the cost of a recording, and this is the cheap end of the scale.
        let mut encoder = GifEncoder::new_with_speed(buf.clone(), 20);
        let _ = encoder.set_repeat(Repeat::Infinite);
        Self { encoder: Some(encoder), buf, held: None, frames: 0, last_delay_ms: 200, size: None }
    }

    /// Adds `frame`, shown from `at`: the one before it is encoded now, for as long as it was on screen.
    pub(crate) fn push(&mut self, frame: RgbaImage, at: Instant) -> Result<(), String> {
        let (w, h) = *self.size.get_or_insert((frame.width(), frame.height()));
        let frame = if frame.dimensions() == (w, h) { frame } else { image::imageops::resize(&frame, w, h, FilterType::Triangle) };
        if let Some((prev, prev_at)) = self.held.take() {
            let ms = at.saturating_duration_since(prev_at).as_millis() as u64;
            self.encode(prev, ms)?;
        }
        self.held = Some((frame, at));
        Ok(())
    }

    fn encode(&mut self, frame: RgbaImage, ms: u64) -> Result<(), String> {
        let ms = ms.max(MIN_DELAY_MS);
        self.last_delay_ms = ms;
        let delay = Delay::from_numer_denom_ms(ms.min(u64::from(u32::MAX)) as u32, 1);
        self.encoder
            .as_mut()
            .ok_or_else(|| "the recording is already finished".to_string())?
            .encode_frame(Frame::from_parts(frame, 0, 0, delay))
            .map_err(|e| format!("encode a frame: {e}"))?;
        self.frames += 1;
        Ok(())
    }

    /// What has been written so far, plus the frame held back (about to be encoded).
    pub(crate) fn bytes_so_far(&self) -> usize {
        self.buf.len()
    }

    pub(crate) fn frames(&self) -> u32 {
        self.frames + u32::from(self.held.is_some())
    }

    /// The last frame stays for as long as the one before it did; the file is returned.
    pub(crate) fn finish(mut self) -> Result<(Vec<u8>, u32), String> {
        if let Some((last, _)) = self.held.take() {
            let ms = self.last_delay_ms;
            self.encode(last, ms)?;
        }
        // The trailer is written when the encoder is dropped.
        drop(self.encoder.take());
        if self.frames == 0 {
            return Err("no frame was captured".to_string());
        }
        Ok((self.buf.take(), self.frames))
    }
}

/// A host capture as a frame: decoded, scaled to [`FRAME_WIDTH`] if wider, and opaque (GIF has no partial alpha).
pub(crate) fn frame_from(bytes: &[u8]) -> Result<RgbaImage, String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("read the capture: {e}"))?;
    let mut rgba = if img.width() > FRAME_WIDTH {
        let h = ((u64::from(img.height()) * u64::from(FRAME_WIDTH)) / u64::from(img.width())).max(1) as u32;
        image::imageops::resize(&img.to_rgba8(), FRAME_WIDTH, h, FilterType::Triangle)
    } else {
        img.to_rgba8()
    };
    for p in rgba.pixels_mut() {
        p.0[3] = 255;
    }
    Ok(rgba)
}

/// `fps` as asked for, held to the limits; `seconds` likewise.
pub(crate) fn limits(fps: Option<u32>, seconds: Option<u32>) -> (u32, u32) {
    (fps.unwrap_or(DEFAULT_FPS).clamp(1, MAX_FPS), seconds.unwrap_or(MAX_SECONDS).clamp(1, MAX_SECONDS))
}

/// One recording in progress, or ended and not yet collected.
struct Recording {
    /// Tells the capture loop to finish with what it has.
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<Finished, String>>,
}

/// One recording a caller at a time.
static RECORDINGS: LazyLock<Mutex<HashMap<String, Recording>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn dir() -> std::path::PathBuf {
    crate::backend::base::get_mux_data_dir().join("tmp/ui-recordings")
}

fn prune_old(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("gif") {
            continue;
        }
        let aged = entry.metadata().and_then(|m| m.modified()).ok().and_then(|m| now.duration_since(m).ok());
        if aged.is_some_and(|age| age > RETENTION) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// One frame: the host's capture of `block_id` as JPEG, blacked out where it has secret fields.
async fn capture_frame(state: &AppState, host: &super::HostIpc, block_id: &str) -> Result<RgbaImage, String> {
    let resp = proxy_to_host_timeout(
        state,
        host,
        "screenshot",
        json!({ "block_id": block_id, "format": "jpeg", "quality": 60 }),
        Some(FRAME_TIMEOUT),
    )
    .await?;
    tokio::task::spawn_blocking(move || frame_of_response(&resp)).await.map_err(|e| e.to_string())?
}

/// The host's answer to one capture as a frame, with what the host found to hide blacked out first, exactly as
/// `screenshot` does. A capture the host could not check for secret fields is an error, so the frame is left out.
pub(crate) fn frame_of_response(resp: &serde_json::Value) -> Result<RgbaImage, String> {
    if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown host error").to_string());
    }
    let encoded = resp.pointer("/data/png_base64").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let bytes = base64::engine::general_purpose::STANDARD.decode(&encoded).map_err(|e| format!("decode the capture: {e}"))?;
    let (bytes, _) = super::screenshot_redact::redact_host_screenshot(resp, bytes, encoded)?;
    frame_from(&bytes)
}

/// The capture loop: a frame every `1/fps` second until stopped, out of time, out of room, or failing.
async fn run(
    state: AppState,
    host: super::HostIpc,
    block_id: String,
    fps: u32,
    seconds: u32,
    mut stop: oneshot::Receiver<()>,
    first: RgbaImage,
    started: Instant,
) -> Result<Finished, String> {
    let every = Duration::from_millis(1000 / u64::from(fps));
    let mut first_builder = GifBuilder::new();
    first_builder.push(first, started)?;
    let mut builder = Some(first_builder);
    let (mut skipped, mut failed_in_a_row) = (0u32, 0u32);
    let mut last_error = String::new();
    let ended = loop {
        let frame_started = Instant::now();
        match capture_frame(&state, &host, &block_id).await {
            Ok(frame) => {
                failed_in_a_row = 0;
                let b = builder.take().ok_or("the recording is finished")?;
                let (b, pushed) = tokio::task::spawn_blocking(move || {
                    let mut b = b;
                    let r = b.push(frame, frame_started);
                    (b, r)
                })
                .await
                .map_err(|e| e.to_string())?;
                builder = Some(b);
                pushed?;
            }
            Err(e) => {
                skipped += 1;
                failed_in_a_row += 1;
                last_error = e;
                if failed_in_a_row >= MAX_FAILED_IN_A_ROW {
                    break "capture failed";
                }
            }
        }
        if builder.as_ref().is_some_and(|b| b.bytes_so_far() >= MAX_BYTES) {
            break "size limit";
        }
        if started.elapsed() >= Duration::from_secs(u64::from(seconds)) {
            break "time limit";
        }
        let wait = every.saturating_sub(frame_started.elapsed());
        tokio::select! {
            _ = &mut stop => break "stopped",
            _ = tokio::time::sleep(wait) => {}
        }
    };
    let elapsed = started.elapsed();
    let builder = builder.take().ok_or("the recording is finished")?;
    let (bytes, frames) = tokio::task::spawn_blocking(move || builder.finish())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| if last_error.is_empty() { e } else { format!("{e} ({last_error})") })?;
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create the recordings folder: {e}"))?;
    let path = dir.join(format!("{}.gif", uuid::Uuid::new_v4()));
    std::fs::write(&path, &bytes).map_err(|e| format!("write the recording: {e}"))?;
    prune_old(&dir);
    Ok(Finished { path, frames, skipped, seconds: elapsed.as_secs_f32(), ended })
}

fn response(status: &str, message: String, finished: Option<&Finished>) -> Response {
    let body = UiRecordResponse {
        status: status.to_string(),
        message,
        path: finished.map(|f| f.path.to_string_lossy().to_string()),
        frames: finished.map_or(0, |f| f.frames),
        skipped: finished.map_or(0, |f| f.skipped),
        seconds: finished.map_or(0.0, |f| f.seconds),
    };
    (StatusCode::OK, Json(body)).into_response()
}

/// `POST /api/v1/ui/record` — backs `UI` action `record`. `op` `start` begins a recording of the caller's
/// own pane (or a browser pane it opened, `pane`); `op` `stop` ends it and returns the file.
pub(crate) async fn handle_ui_record(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiRecordRequest>,
) -> impl IntoResponse {
    let who = req.auth.agent_id.clone();
    match req.op.as_str() {
        "start" => {
            let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
                Ok(b) => b,
                Err((code, e)) => return err_response(code, e),
            };
            let host = match get_host_ipc(&state).await {
                Ok(h) => h,
                Err(e) => return err_response(StatusCode::SERVICE_UNAVAILABLE, e),
            };
            let (fps, seconds) = limits(req.fps, req.seconds);
            if RECORDINGS.lock().unwrap_or_else(|e| e.into_inner()).get(&who).is_some_and(|r| !r.task.is_finished()) {
                return err_response(
                    StatusCode::CONFLICT,
                    "you are already recording: stop that recording first (UI record, op stop)".to_string(),
                );
            }
            // The first frame is taken here, so a pane that can't be captured (its window minimized or hidden)
            // fails the start instead of a recording of nothing.
            let started = Instant::now();
            let first = match capture_frame(&state, &host, &block_id).await {
                Ok(f) => f,
                Err(e) => {
                    return err_response(
                        StatusCode::BAD_GATEWAY,
                        format!("couldn't capture the pane, so nothing is recording: {e} (a minimized or hidden window can't be captured)"),
                    )
                }
            };
            let (tx, rx) = oneshot::channel();
            {
                let mut all = RECORDINGS.lock().unwrap_or_else(|e| e.into_inner());
                if all.get(&who).is_some_and(|r| !r.task.is_finished()) {
                    return err_response(
                        StatusCode::CONFLICT,
                        "you are already recording: stop that recording first (UI record, op stop)".to_string(),
                    );
                }
                let task = tokio::spawn(run(state.clone(), host, block_id.clone(), fps, seconds, rx, first, started));
                all.insert(who.clone(), Recording { stop: Some(tx), task });
            }
            tracing::info!(agent_id = %who, block_id = %block_id, fps, seconds, "[ui-automation] record start");
            response(
                "recording",
                format!(
                    "Recording your pane at up to {fps} frames a second for at most {seconds} seconds. Do what you want to show, then call UI record with op stop; it returns the file."
                ),
                None,
            )
        }
        "stop" => {
            // The caller's identity is checked, but not that it is free to act: a recording must always be stoppable.
            if let Err(e) = verified_block_id(&state, caller.as_deref(), &req.auth) {
                return err_response(StatusCode::UNAUTHORIZED, e);
            }
            let Some(mut rec) = RECORDINGS.lock().unwrap_or_else(|e| e.into_inner()).remove(&who) else {
                return err_response(StatusCode::NOT_FOUND, "you have no recording: start one first (UI record, op start)".to_string());
            };
            if let Some(stop) = rec.stop.take() {
                let _ = stop.send(());
            }
            match rec.task.await {
                Ok(Ok(f)) => {
                    tracing::info!(agent_id = %who, path = %f.path.display(), frames = f.frames, skipped = f.skipped, ended = f.ended, "[ui-automation] record stop");
                    let note = if f.skipped > 0 {
                        format!(" {} frame(s) were left out because the page could not be checked for secret fields or a capture failed.", f.skipped)
                    } else {
                        String::new()
                    };
                    let msg = format!(
                        "Recording saved to {} ({} frames, {:.1} s, ended: {}).{note} Look at it before you attach it (Read shows an image; open it with Pane view \"media\" to see it move), then pass its path in files of Tasks update or submit.",
                        f.path.display(),
                        f.frames,
                        f.seconds,
                        f.ended
                    );
                    response("saved", msg, Some(&f))
                }
                Ok(Err(e)) => err_response(StatusCode::BAD_GATEWAY, format!("the recording failed: {e}")),
                Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("the recording stopped unexpectedly: {e}")),
            }
        }
        other => err_response(StatusCode::BAD_REQUEST, format!("op is start or stop, not {other:?}")),
    }
}

#[cfg(test)]
#[path = "ui_record_tests.rs"]
mod tests;
