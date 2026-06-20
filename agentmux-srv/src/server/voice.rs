// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Voice speech-to-text endpoint — `POST /api/v1/voice/transcribe`.
//!
//! The renderer captures mic audio (getUserMedia, gated by the CEF permission
//! handler #1602) and POSTs each silence-bounded utterance here; we forward it
//! to the configured Whisper backend and return the transcript. Keys/paths stay
//! server-side — the renderer never sees them.
//!
//! Web Speech API can't transcribe in CEF (closed-source Google service), so
//! this capture-and-send path is the real engine. See
//! docs/specs/SPEC_VOICE_STT_ENGINE_2026_06_20.md and #1591.
//!
//! Backends (selected by the `voice:engine` setting):
//!   * "groq" (default) — hosted whisper-large-v3-turbo; renderer sends webm.
//!   * "whisper-local"  — offline whisper.cpp via a local `whisper-cli`
//!                        subprocess; renderer sends 16 kHz mono WAV. Opt-in,
//!                        configured via settings/env; needs a model file.

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
///   502 upstream/subprocess error.
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
    let settings = read_settings_json();
    let engine = resolve_engine(&settings);

    let result = if engine == "whisper-local" {
        transcribe_local_whisper(body, lang.as_deref(), &settings).await
    } else {
        match resolve_groq_key(&settings) {
            Some(key) => transcribe_groq(&key, body, &mime, lang.as_deref()).await,
            None => Err(NotConfigured(
                "Groq backend not configured — set voice:groqApiKey in settings.json \
                 or the AGENTMUX_GROQ_API_KEY env var"
                    .to_string(),
            )),
        }
    };

    match result {
        Ok(text) => Json(json!({ "text": text })).into_response(),
        Err(NotConfigured(msg)) => {
            // Mirrors the frontend's `service-not-allowed` UX (#1603): voice
            // isn't usable in this build/config yet.
            (StatusCode::NOT_IMPLEMENTED, Json(json!({ "error": msg }))).into_response()
        }
        Err(Upstream(msg)) => {
            tracing::warn!(target: "voice", "transcription failed: {msg}");
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": msg }))).into_response()
        }
    }
}

/// Transcription failure: distinguishes "not configured" (→ 501, surfaced as the
/// frontend's "unavailable" guidance) from a real upstream/runtime error (→ 502).
enum SttError {
    NotConfigured(String),
    Upstream(String),
}
use SttError::{NotConfigured, Upstream};

// ── Config resolution (server-side only) ────────────────────────────────────

/// Read settings.json once; `None` if absent/unparseable.
fn read_settings_json() -> Option<serde_json::Value> {
    let path = crate::backend::base::get_wave_config_dir().join("settings.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn settings_str(settings: &Option<serde_json::Value>, key: &str) -> Option<String> {
    settings
        .as_ref()?
        .get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `voice:engine` — "groq" (default) or "whisper-local". Env override:
/// `AGENTMUX_VOICE_ENGINE`.
fn resolve_engine(settings: &Option<serde_json::Value>) -> String {
    std::env::var("AGENTMUX_VOICE_ENGINE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| settings_str(settings, "voice:engine"))
        .unwrap_or_else(|| "groq".to_string())
}

/// Groq key: `AGENTMUX_GROQ_API_KEY` env, else settings.json `voice:groqApiKey`.
fn resolve_groq_key(settings: &Option<serde_json::Value>) -> Option<String> {
    if let Ok(k) = std::env::var("AGENTMUX_GROQ_API_KEY") {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Some(k);
        }
    }
    settings_str(settings, "voice:groqApiKey")
}

/// Env-first, then settings.json, for a path-valued config key.
fn resolve_path(settings: &Option<serde_json::Value>, env: &str, key: &str) -> Option<String> {
    std::env::var(env)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| settings_str(settings, key))
}

// ── Groq backend (hosted) ───────────────────────────────────────────────────

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

/// POST the audio to Groq's OpenAI-compatible transcription endpoint.
async fn transcribe_groq(
    key: &str,
    audio: Bytes,
    mime: &str,
    lang: Option<&str>,
) -> Result<String, SttError> {
    let part = reqwest::multipart::Part::bytes(audio.to_vec())
        .file_name(format!("audio.{}", mime_ext(mime)))
        .mime_str(mime)
        .map_err(|e| Upstream(format!("invalid mime: {e}")))?;

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
        .map_err(|e| Upstream(format!("request failed: {e}")))?;

    let status = resp.status();
    let text_body = resp
        .text()
        .await
        .map_err(|e| Upstream(format!("reading response: {e}")))?;

    if !status.is_success() {
        let snippet: String = text_body.chars().take(300).collect();
        return Err(Upstream(format!("groq {}: {}", status.as_u16(), snippet)));
    }

    let v: serde_json::Value =
        serde_json::from_str(&text_body).map_err(|e| Upstream(format!("parsing response: {e}")))?;
    Ok(v.get("text")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .trim()
        .to_string())
}

// ── Local whisper.cpp backend (offline, opt-in) ─────────────────────────────

/// Transcribe via a local `whisper-cli` (whisper.cpp) subprocess. The renderer
/// sends 16 kHz mono WAV for this engine, so we write the body to a temp `.wav`
/// and run the CLI. Fully offline; nothing leaves the machine.
///
/// Config (env-first):
///   * binary — `AGENTMUX_WHISPER_CLI`  / settings `voice:whisperCliPath`
///   * model  — `AGENTMUX_WHISPER_MODEL` / settings `voice:whisperModelPath`
///
/// On-demand model/binary download is a follow-up (#1591); for now both paths
/// are user-provided. Missing config → NotConfigured (501).
async fn transcribe_local_whisper(
    audio: Bytes,
    lang: Option<&str>,
    settings: &Option<serde_json::Value>,
) -> Result<String, SttError> {
    let cli = resolve_path(settings, "AGENTMUX_WHISPER_CLI", "voice:whisperCliPath").ok_or_else(
        || {
            NotConfigured(
                "local whisper not configured — set voice:whisperCliPath (path to whisper-cli) \
                 in settings.json or AGENTMUX_WHISPER_CLI"
                    .to_string(),
            )
        },
    )?;
    let model = resolve_path(settings, "AGENTMUX_WHISPER_MODEL", "voice:whisperModelPath")
        .ok_or_else(|| {
            NotConfigured(
                "local whisper not configured — set voice:whisperModelPath (path to a GGML model) \
                 in settings.json or AGENTMUX_WHISPER_MODEL"
                    .to_string(),
            )
        })?;
    if !std::path::Path::new(&cli).exists() {
        return Err(NotConfigured(format!("whisper-cli not found at {cli}")));
    }
    if !std::path::Path::new(&model).exists() {
        return Err(NotConfigured(format!("whisper model not found at {model}")));
    }

    // Write the WAV body to a unique temp file. Use a monotonic-ish unique name
    // (nanos + a counter) — the sidecar handles many requests.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let wav_path = std::env::temp_dir().join(format!("agentmux-voice-{ts}.wav"));
    std::fs::write(&wav_path, &audio).map_err(|e| Upstream(format!("temp write: {e}")))?;

    // whisper-cli: -nt no timestamps, -np no progress prints → transcript on
    // stdout; logs go to stderr.
    let mut cmd = tokio::process::Command::new(&cli);
    cmd.arg("-m").arg(&model).arg("-f").arg(&wav_path).arg("-nt").arg("-np");
    if let Some(l) = lang {
        cmd.arg("-l").arg(l);
    }

    let output = cmd.output().await;
    let _ = std::fs::remove_file(&wav_path); // best-effort cleanup

    let output = output.map_err(|e| Upstream(format!("spawn whisper-cli: {e}")))?;
    if !output.status.success() {
        let err: String = String::from_utf8_lossy(&output.stderr).chars().take(300).collect();
        return Err(Upstream(format!(
            "whisper-cli exited {}: {}",
            output.status.code().unwrap_or(-1),
            err
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

    #[test]
    fn engine_defaults_to_groq() {
        assert_eq!(resolve_engine(&None), "groq");
        let s = serde_json::json!({ "voice:engine": "whisper-local" });
        assert_eq!(resolve_engine(&Some(s)), "whisper-local");
    }
}
