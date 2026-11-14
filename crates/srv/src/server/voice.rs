// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Voice speech-to-text endpoint — `POST /api/v1/voice/transcribe`, and `GET /api/v1/voice/status`, which
//! says whether the chosen engine has what it needs, so the mic can say so before it records.
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

#[cfg(windows)]
use agentmux_common::win32::NoWindow;
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde_json::json;

use super::AppState;
use crate::backend::vault;

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
    State(state): State<AppState>,
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
        // The key may be a Vault entry, opened here for this one request and sent nowhere else.
        let (keyed, store) = (settings.clone(), state.identity_store.clone());
        let key = tokio::task::spawn_blocking(move || groq_key(&keyed, Some(&store))).await.unwrap_or(GroqKey::Gone);
        match key {
            GroqKey::Found(key) => transcribe_groq(&key, body, &mime, lang.as_deref()).await,
            GroqKey::Gone => Err(NotConfigured(GROQ_GONE.to_string())),
            GroqKey::Unset => Err(NotConfigured(GROQ_NOT_CONFIGURED.to_string())),
        }
    };

    match result {
        Ok(text) => Json(json!({ "text": text })).into_response(),
        Err(NotConfigured(msg)) => {
            // Mirrors the frontend's `service-not-allowed` UX (#1603): voice
            // isn't usable in this build/config yet.
            log_not_configured(&msg);
            (StatusCode::NOT_IMPLEMENTED, Json(json!({ "error": msg }))).into_response()
        }
        Err(Upstream(msg)) => {
            tracing::warn!(target: "voice", "transcription failed: {msg}");
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": msg }))).into_response()
        }
    }
}

const GROQ_NOT_CONFIGURED: &str = "Groq backend not configured — choose a Vault key for it in Settings > Recording \
     (add one to the Vault there), or set the AGENTMUX_GROQ_API_KEY environment variable";
const GROQ_GONE: &str = "The Vault key chosen for voice input is not there, or is not an API key — choose another in \
     Settings > Recording, or set the AGENTMUX_GROQ_API_KEY environment variable";

/// `GET /api/v1/voice/status` → `{ "engine": "groq", "ready": false, "missing": "groq-key", "error": "..." }`.
/// Whether the chosen engine has what it needs to transcribe, read from the settings and the environment
/// alone: nothing is sent anywhere, no program is run and no model is fetched. The mic asks this before it
/// records, so a person with no engine set up is told at the click, not after speaking into nothing.
pub(super) async fn handle_voice_status(State(state): State<AppState>) -> impl IntoResponse {
    let settings = read_settings_json();
    let engine = resolve_engine(&settings);
    let gap = setup_gap(&settings, &engine, Some(&state.identity_store));
    if let Some(gap) = &gap {
        log_not_configured(&gap.error);
    }
    Json(json!({
        "engine": engine,
        "ready": gap.is_none(),
        "missing": gap.as_ref().map(|g| g.missing),
        "error": gap.map(|g| g.error),
    }))
}

/// What an engine still needs before it can transcribe.
#[derive(Debug, PartialEq)]
struct SetupGap {
    /// `groq-key`, `whisper-cli` or `whisper-model`: what the pane's own wording is chosen by.
    missing: &'static str,
    /// The same sentence a transcription refused for it answers with.
    error: String,
}

/// The first thing `engine` still needs, or `None` when a transcription can be tried. For local whisper a
/// model that is fetched on first use is not a gap: only a model path that was given and is not there.
fn setup_gap(settings: &Option<serde_json::Value>, engine: &str, store: Option<&crate::backend::storage::store::Store>) -> Option<SetupGap> {
    setup_gap_with(env_groq_key(), settings, engine, store)
}

fn setup_gap_with(groq_env: Option<String>, settings: &Option<serde_json::Value>, engine: &str, store: Option<&crate::backend::storage::store::Store>) -> Option<SetupGap> {
    let gap = |missing, error: SttError| {
        let (NotConfigured(error) | Upstream(error)) = error;
        Some(SetupGap { missing, error })
    };
    if engine != "whisper-local" {
        // The Vault entry is looked for, never opened: the status runs whenever the mic is clicked.
        return match groq_key_state_with(groq_env, settings, store) {
            GroqKeyState::Ready => None,
            GroqKeyState::Gone => gap("groq-key", NotConfigured(GROQ_GONE.to_string())),
            GroqKeyState::Unset => gap("groq-key", NotConfigured(GROQ_NOT_CONFIGURED.to_string())),
        };
    }
    if let Err(e) = resolve_whisper_cli(settings) {
        return gap("whisper-cli", e);
    }
    match explicit_model_path(settings) {
        Some(Err(e)) => gap("whisper-model", e),
        Some(Ok(_)) => None,
        None => model_name(settings).err().and_then(|e| gap("whisper-model", e)),
    }
}

/// One line in the log per run for a voice request that found no engine set up. The answer goes to the
/// window, which is not in any log: without this line a report of "the mic does nothing" could not be read
/// from the logs at all.
fn log_not_configured(msg: &str) {
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!(
            target: "voice",
            "voice input was asked for, but its transcription engine is not set up: {msg} (logged once per run)"
        );
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

/// Read settings.json once; `None` if absent/unparseable. The file the config
/// watcher loads, not `<config dir>/settings.json`: on a shared channel those
/// differ (#1903).
fn read_settings_json() -> Option<serde_json::Value> {
    let path = crate::backend::config_watcher_fs::resolve_settings_dir().join("settings.json");
    let content = std::fs::read_to_string(path).ok()?;
    agentmux_common::settings_jsonc::parse_jsonc(&content)
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

/// What the Groq key setting comes to.
enum GroqKey {
    Found(zeroize::Zeroizing<String>),
    /// The setting names a Vault entry that is not there (or is not an API key).
    Gone,
    Unset,
}

/// Groq key: `AGENTMUX_GROQ_API_KEY` env (it wins, for CI and scripts), else settings.json `voice:groqApiKey`:
/// a `vault:<entry id>` reference, read from the Vault here, or a plain key from before it was moved.
fn groq_key(settings: &Option<serde_json::Value>, store: Option<&crate::backend::storage::store::Store>) -> GroqKey {
    groq_key_with(env_groq_key(), settings, store)
}

fn groq_key_with(env: Option<String>, settings: &Option<serde_json::Value>, store: Option<&crate::backend::storage::store::Store>) -> GroqKey {
    if let Some(k) = env {
        return GroqKey::Found(zeroize::Zeroizing::new(k));
    }
    let Some(set) = settings_str(settings, "voice:groqApiKey") else { return GroqKey::Unset };
    match (vault::setting_key::entry_of(&set), store) {
        (Some(id), Some(store)) => match vault::setting_key::read(store, id, "AgentMux voice input", "voice input") {
            Ok(v) => GroqKey::Found(v),
            Err(_) => GroqKey::Gone,
        },
        (Some(_), None) => GroqKey::Gone,
        (None, _) => GroqKey::Found(zeroize::Zeroizing::new(set)),
    }
}

fn env_groq_key() -> Option<String> {
    let k = std::env::var("AGENTMUX_GROQ_API_KEY").ok()?.trim().to_string();
    (!k.is_empty()).then_some(k)
}

enum GroqKeyState {
    Ready,
    Gone,
    Unset,
}

/// Whether there is a Groq key to use, without opening a Vault entry.
fn groq_key_state_with(env: Option<String>, settings: &Option<serde_json::Value>, store: Option<&crate::backend::storage::store::Store>) -> GroqKeyState {
    if env.is_some() {
        return GroqKeyState::Ready;
    }
    let Some(set) = settings_str(settings, "voice:groqApiKey") else { return GroqKeyState::Unset };
    match vault::setting_key::entry_of(&set) {
        Some(id) if store.is_some_and(|s| vault::setting_key::present(s, id)) => GroqKeyState::Ready,
        Some(_) => GroqKeyState::Gone,
        None => GroqKeyState::Ready,
    }
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
///   * binary — `AGENTMUX_WHISPER_CLI` / settings `voice:whisperCliPath`
///              (user-provided; auto-bundling is a follow-up — per-platform
///              binary fetch is fragile)
///   * model  — auto-downloaded on first use (default `base.en`, configurable
///              via `voice:whisperModel`); override with an explicit
///              `voice:whisperModelPath` / `AGENTMUX_WHISPER_MODEL`.
///
/// Missing binary → NotConfigured (501).
/// Default GGML model name when none is configured. base.en (~142 MB) is a good
/// accuracy/size balance for English; configurable via `voice:whisperModel`.
const DEFAULT_WHISPER_MODEL: &str = "base.en";
/// Cap the one-time model download so a stuck transfer can't wedge a request.
const MODEL_DOWNLOAD_TIMEOUT_SECS: u64 = 600;

/// Resolve the GGML model path, downloading an auto-managed model on first use.
///
///   * Explicit `voice:whisperModelPath` / `AGENTMUX_WHISPER_MODEL` → used as-is
///     (no download; error if missing).
///   * Otherwise → `<config>/whisper-models/ggml-<name>.bin`, where `name` comes
///     from `voice:whisperModel` (default `base.en`), fetched once from the
///     whisper.cpp model repo. A global lock serializes concurrent first-uses.
async fn ensure_local_model(
    settings: &Option<serde_json::Value>,
) -> Result<std::path::PathBuf, SttError> {
    if let Some(explicit) = explicit_model_path(settings) {
        return explicit;
    }
    let name = model_name(settings)?;

    let dir = crate::backend::base::get_mux_config_dir().join("whisper-models");
    let path = dir.join(format!("ggml-{name}.bin"));
    if path.exists() {
        return Ok(path);
    }

    // Single global lock: serializes ALL first-use model downloads (not
    // per-model), so one model's download blocks any other model's first-use
    // for up to the 600s cap. Acceptable — first-use downloads are rare and
    // models are typically not switched mid-session.
    static DL_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _guard = DL_LOCK.get_or_init(|| tokio::sync::Mutex::new(())).lock().await;
    if path.exists() {
        return Ok(path); // another request finished the download while we waited
    }

    std::fs::create_dir_all(&dir).map_err(|e| Upstream(format!("create models dir: {e}")))?;
    let url =
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin");
    tracing::info!(target: "voice", "downloading whisper model '{name}' from {url}");

    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(MODEL_DOWNLOAD_TIMEOUT_SECS),
        async {
            let resp = reqwest::Client::new()
                .get(&url)
                .send()
                .await
                .map_err(|e| Upstream(format!("model download request: {e}")))?;
            if !resp.status().is_success() {
                return Err(Upstream(format!(
                    "model download {} for '{name}'",
                    resp.status().as_u16()
                )));
            }
            resp.bytes()
                .await
                .map_err(|e| Upstream(format!("model download body: {e}")))
        },
    )
    .await
    .map_err(|_| Upstream(format!("model download timed out after {MODEL_DOWNLOAD_TIMEOUT_SECS}s")))??;

    // Write to a temp file then rename so a partial download never looks valid.
    let tmp = dir.join(format!("ggml-{name}.bin.part"));
    std::fs::write(&tmp, &bytes).map_err(|e| Upstream(format!("model write: {e}")))?;
    std::fs::rename(&tmp, &path).map_err(|e| Upstream(format!("model finalize: {e}")))?;
    tracing::info!(target: "voice", "whisper model '{name}' ready ({} bytes)", bytes.len());
    Ok(path)
}

/// The model file the settings or the environment name outright, if they name one: there, or an error.
fn explicit_model_path(
    settings: &Option<serde_json::Value>,
) -> Option<Result<std::path::PathBuf, SttError>> {
    let p = resolve_path(settings, "AGENTMUX_WHISPER_MODEL", "voice:whisperModelPath")?;
    let pb = std::path::PathBuf::from(&p);
    Some(if pb.exists() {
        Ok(pb)
    } else {
        Err(NotConfigured(format!("whisper model not found at {p}")))
    })
}

/// The name of the auto-managed model (`voice:whisperModel`, default `base.en`).
fn model_name(settings: &Option<serde_json::Value>) -> Result<String, SttError> {
    let name = settings_str(settings, "voice:whisperModel")
        .unwrap_or_else(|| DEFAULT_WHISPER_MODEL.to_string());
    // Sanitize: model names are simple tokens — reject anything that could be a
    // path or URL component (defends the format!-built path and URL it is put in).
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) {
        return Err(NotConfigured(format!("invalid voice:whisperModel name: {name}")));
    }
    Ok(name)
}

/// The `whisper-cli` program the local engine runs: its path, when one is set and is there.
fn resolve_whisper_cli(settings: &Option<serde_json::Value>) -> Result<String, SttError> {
    let cli = resolve_path(settings, "AGENTMUX_WHISPER_CLI", "voice:whisperCliPath").ok_or_else(
        || {
            NotConfigured(
                "local whisper not configured — set voice:whisperCliPath (path to whisper-cli) \
                 in settings.json or AGENTMUX_WHISPER_CLI"
                    .to_string(),
            )
        },
    )?;
    if !std::path::Path::new(&cli).exists() {
        return Err(NotConfigured(format!("whisper-cli not found at {cli}")));
    }
    Ok(cli)
}

async fn transcribe_local_whisper(
    audio: Bytes,
    lang: Option<&str>,
    settings: &Option<serde_json::Value>,
) -> Result<String, SttError> {
    let cli = resolve_whisper_cli(settings)?;
    // Resolve the model: an explicit path wins, otherwise an auto-managed model
    // is downloaded once to the config dir (zero-config for the model file).
    let model = ensure_local_model(settings).await?;

    // Write the WAV body to a unique temp file. Timestamp alone can collide
    // under concurrent requests (coarse Windows SystemTime granularity), so add
    // a process-global atomic counter to guarantee uniqueness.
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let wav_path = std::env::temp_dir().join(format!("agentmux-voice-{ts}-{seq}.wav"));
    std::fs::write(&wav_path, &audio).map_err(|e| Upstream(format!("temp write: {e}")))?;

    // whisper-cli: -nt no timestamps, -np no progress prints → transcript on
    // stdout; logs go to stderr.
    let mut cmd = tokio::process::Command::new(&cli);
    // Third-party transcription CLI (whisper.cpp).
    crate::backend::pane_env::sanitize_external_command(&mut cmd);
    cmd.arg("-m").arg(&model).arg("-f").arg(&wav_path).arg("-nt").arg("-np");
    if let Some(l) = lang {
        cmd.arg("-l").arg(l);
    }
    // Bound the run: a wedged CLI (corrupt model, pathological input) must not
    // block the HTTP handler forever. kill_on_drop ensures the timed-out child
    // is reaped when the timeout future drops it.
    cmd.kill_on_drop(true);
    // CREATE_NO_WINDOW: console-flash suppression, see crates/common/src/cli.rs
    #[cfg(windows)]
    {
        cmd.no_window();
    }
    const WHISPER_TIMEOUT_SECS: u64 = 120;

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(WHISPER_TIMEOUT_SECS),
        cmd.output(),
    )
    .await;
    let _ = std::fs::remove_file(&wav_path); // best-effort cleanup

    let output = match output {
        Err(_) => {
            return Err(Upstream(format!(
                "whisper-cli timed out after {WHISPER_TIMEOUT_SECS}s"
            )))
        }
        Ok(r) => r.map_err(|e| Upstream(format!("spawn whisper-cli: {e}")))?,
    };
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

    /// These read the settings alone; a machine with the engine's own environment variables set would
    /// answer for those instead, so such a run is skipped.
    fn env_is_clean() -> bool {
        ["AGENTMUX_GROQ_API_KEY", "AGENTMUX_WHISPER_CLI", "AGENTMUX_WHISPER_MODEL"]
            .iter()
            .all(|k| std::env::var(k).is_err())
    }

    fn vault_with_key(value: &str) -> (crate::backend::storage::store::Store, String) {
        crate::backend::vault::seal::use_test_key();
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        store.apply_identity_schema_for_tests().unwrap();
        let id = vault::setting_key::keep(&store, "Groq key (voice input)", "gsk_from_the_vault").unwrap();
        (store, id)
    }

    fn setting(v: &str) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "voice:groqApiKey": v }))
    }

    #[test]
    fn a_vault_reference_is_read_from_the_vault_and_a_gone_one_says_so() {
        let (store, id) = vault_with_key("x");
        let GroqKey::Found(k) = groq_key_with(None, &setting(&format!("vault:{id}")), Some(&store)) else { panic!("not found") };
        assert_eq!(k.as_str(), "gsk_from_the_vault");
        assert!(matches!(groq_key_state_with(None, &setting(&format!("vault:{id}")), Some(&store)), GroqKeyState::Ready));
        assert!(matches!(groq_key_with(None, &setting("vault:nope"), Some(&store)), GroqKey::Gone));
        assert!(matches!(groq_key_with(None, &setting(&format!("vault:{id}")), None), GroqKey::Gone), "no Vault to open");
        let gap = setup_gap_with(None, &setting("vault:nope"), "groq", Some(&store)).expect("a dangling reference is a gap");
        assert_eq!((gap.missing, gap.error.contains("not there")), ("groq-key", true), "{}", gap.error);
        assert!(!gap.error.contains("nope"), "the answer names no entry: {}", gap.error);
    }

    #[test]
    fn the_environment_variable_still_wins_over_the_setting_and_the_vault() {
        let (store, id) = vault_with_key("x");
        let GroqKey::Found(k) = groq_key_with(Some("gsk_from_env".into()), &setting(&format!("vault:{id}")), Some(&store)) else { panic!("not found") };
        assert_eq!(k.as_str(), "gsk_from_env");
        // Even a dangling reference is not a gap when the environment has the key.
        assert!(matches!(groq_key_state_with(Some("gsk_from_env".into()), &setting("vault:nope"), Some(&store)), GroqKeyState::Ready));
        assert!(matches!(groq_key_with(None, &None, Some(&store)), GroqKey::Unset));
    }

    #[test]
    fn a_plain_key_left_in_the_settings_still_works_until_it_is_moved() {
        let GroqKey::Found(k) = groq_key_with(None, &setting("gsk_plain"), None) else { panic!("not found") };
        assert_eq!(k.as_str(), "gsk_plain");
    }

    #[test]
    fn groq_with_no_key_is_a_gap_and_with_one_is_ready() {
        if !env_is_clean() {
            return;
        }
        let gap = setup_gap(&None, "groq", None).expect("no settings, no key");
        assert_eq!(gap.missing, "groq-key");
        assert!(gap.error.contains("Vault"), "{}", gap.error);
        // A blank key is no key.
        let blank = Some(serde_json::json!({ "voice:groqApiKey": "  " }));
        assert_eq!(setup_gap(&blank, "groq", None).map(|g| g.missing), Some("groq-key"));
        let keyed = Some(serde_json::json!({ "voice:groqApiKey": "gsk_x" }));
        assert_eq!(setup_gap(&keyed, "groq", None), None);
    }

    #[test]
    fn local_whisper_needs_its_program_and_a_named_model_file() {
        if !env_is_clean() {
            return;
        }
        assert_eq!(setup_gap(&None, "whisper-local", None).map(|g| g.missing), Some("whisper-cli"));
        let gone = Some(serde_json::json!({ "voice:whisperCliPath": "/no/such/whisper-cli" }));
        let gap = setup_gap(&gone, "whisper-local", None).expect("the program is not there");
        assert_eq!(gap.missing, "whisper-cli");
        assert!(gap.error.contains("/no/such/whisper-cli"), "{}", gap.error);

        // Any file that is there stands for the program: the status does not run it.
        let here = std::env::current_exe().unwrap().to_string_lossy().to_string();
        // The default model is fetched on first use, so it is not a gap.
        let ready = Some(serde_json::json!({ "voice:whisperCliPath": here }));
        assert_eq!(setup_gap(&ready, "whisper-local", None), None);
        let model_gone = Some(serde_json::json!({
            "voice:whisperCliPath": here, "voice:whisperModelPath": "/no/such/model.bin",
        }));
        assert_eq!(setup_gap(&model_gone, "whisper-local", None).map(|g| g.missing), Some("whisper-model"));
        let bad_name = Some(serde_json::json!({
            "voice:whisperCliPath": here, "voice:whisperModel": "../etc/passwd",
        }));
        assert_eq!(setup_gap(&bad_name, "whisper-local", None).map(|g| g.missing), Some("whisper-model"));
    }

    #[test]
    fn engine_defaults_to_groq() {
        assert_eq!(resolve_engine(&None), "groq");
        let s = serde_json::json!({ "voice:engine": "whisper-local" });
        assert_eq!(resolve_engine(&Some(s)), "whisper-local");
    }
}
