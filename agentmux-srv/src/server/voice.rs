// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Voice speech-to-text endpoint — `POST /api/v1/voice/transcribe`.
//!
//! The renderer captures mic audio (getUserMedia → MediaRecorder, gated by the
//! CEF permission handler #1602) and POSTs each silence-bounded utterance here
//! as a raw audio body. We forward it to a Whisper backend and return the
//! transcript. The API key stays server-side — the renderer never sees it.
//!
//! Web Speech API can't transcribe in CEF (closed-source Google service,
//! Chrome-build-bound), so this capture-and-send path is the real engine. See
//! docs/specs/SPEC_VOICE_STT_ENGINE_2026_06_20.md and #1591.
//!
//! PR 1 ships the Groq hosted backend (whisper-large-v3-turbo) as free
//! functions; the pluggable `SttBackend` trait + local whisper.cpp land in PR 2.

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde_json::json;

use super::AppState;

const GROQ_TRANSCRIBE_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const GROQ_MODEL: &str = "whisper-large-v3-turbo";

#[derive(serde::Deserialize)]
pub(super) struct TranscribeQuery {
    /// MIME type of the posted audio (e.g. "audio/webm"). Defaults to webm.
    mime: Option<String>,
    /// Optional BCP-47 language hint (e.g. "en"); improves accuracy + latency.
    lang: Option<String>,
}

/// `POST /api/v1/voice/transcribe?mime=audio/webm&lang=en` — body is raw audio.
/// → 200 `{ "text": "..." }` · 400 empty body · 501 no backend configured ·
///   502 upstream error.
pub(super) async fn handle_voice_transcribe(
    State(_state): State<AppState>,
    Query(q): Query<TranscribeQuery>,
    body: Bytes,
) -> impl IntoResponse {
    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "empty audio body" })))
            .into_response();
    }

    let mime = q.mime.unwrap_or_else(|| "audio/webm".to_string());
    let lang = q.lang.filter(|l| !l.trim().is_empty());

    let Some(key) = resolve_groq_key() else {
        // Mirrors the frontend's `service-not-allowed` UX (#1603): voice isn't
        // configured in this build yet.
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "error": "no STT backend configured — set voice:groqApiKey in \
                          settings.json or the AGENTMUX_GROQ_API_KEY env var"
            })),
        )
            .into_response();
    };

    match transcribe_groq(&key, body, &mime, lang.as_deref()).await {
        Ok(text) => Json(json!({ "text": text })).into_response(),
        Err(e) => {
            tracing::warn!(target: "voice", "transcription failed: {e}");
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response()
        }
    }
}

/// Resolve the Groq API key, server-side only:
///   1. `AGENTMUX_GROQ_API_KEY` env var
///   2. settings.json key `voice:groqApiKey`
fn resolve_groq_key() -> Option<String> {
    if let Ok(k) = std::env::var("AGENTMUX_GROQ_API_KEY") {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Some(k);
        }
    }
    let path = crate::backend::base::get_wave_config_dir().join("settings.json");
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("voice:groqApiKey")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// File extension matching the posted MIME, so Groq's content sniffing accepts
/// the multipart `file` part.
fn mime_ext(mime: &str) -> &'static str {
    let m = mime.to_ascii_lowercase();
    if m.contains("webm") {
        "webm"
    } else if m.contains("ogg") {
        "ogg"
    } else if m.contains("wav") {
        "wav"
    } else if m.contains("mp4") || m.contains("m4a") {
        "m4a"
    } else if m.contains("mpeg") || m.contains("mp3") {
        "mp3"
    } else if m.contains("flac") {
        "flac"
    } else {
        "webm"
    }
}

/// POST the audio to Groq's OpenAI-compatible transcription endpoint and return
/// the transcript text. ~$0.0007/min, ~216× real-time.
async fn transcribe_groq(
    key: &str,
    audio: Bytes,
    mime: &str,
    lang: Option<&str>,
) -> Result<String, String> {
    let part = reqwest::multipart::Part::bytes(audio.to_vec())
        .file_name(format!("audio.{}", mime_ext(mime)))
        .mime_str(mime)
        .map_err(|e| format!("invalid mime: {e}"))?;

    let mut form = reqwest::multipart::Form::new()
        .text("model", GROQ_MODEL)
        .text("response_format", "json")
        .part("file", part);
    if let Some(l) = lang {
        form = form.text("language", l.to_string());
    }

    let resp = reqwest::Client::new()
        .post(GROQ_TRANSCRIBE_URL)
        .bearer_auth(key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = resp.status();
    let text_body = resp
        .text()
        .await
        .map_err(|e| format!("reading response: {e}"))?;

    if !status.is_success() {
        // Don't leak the key; cap the upstream body in the message.
        let snippet: String = text_body.chars().take(300).collect();
        return Err(format!("groq {}: {}", status.as_u16(), snippet));
    }

    let v: serde_json::Value =
        serde_json::from_str(&text_body).map_err(|e| format!("parsing response: {e}"))?;
    Ok(v.get("text")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_ext_maps_common_types() {
        assert_eq!(mime_ext("audio/webm;codecs=opus"), "webm");
        assert_eq!(mime_ext("audio/ogg"), "ogg");
        assert_eq!(mime_ext("audio/wav"), "wav");
        assert_eq!(mime_ext("audio/mp4"), "m4a");
        assert_eq!(mime_ext("audio/mpeg"), "mp3");
        assert_eq!(mime_ext("application/octet-stream"), "webm"); // fallback
    }
}
