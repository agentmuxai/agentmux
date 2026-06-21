# SPEC: Voice STT engine — capture-and-send to Whisper

**Date:** 2026-06-20
**Author:** Claude (Opus 4.8)
**Status:** In progress (PR 1)
**Tracking:** #1591 §4c/4d. Builds on the merged foundation: CEF mic-permission
handler (#1602) + actionable permission UX (#1603). Supersedes the Web Speech
engine, which **cannot transcribe in CEF** (closed-source Google speech service,
Chrome-build-bound — see `agentmux-voice-permissions-report.md`).

---

## 1. Goal

Make voice input actually transcribe in packaged AgentMux builds by replacing
the (dead-in-CEF) Web Speech recognizer with a **capture-and-send** pipeline:
the renderer captures mic audio and sends it to `agentmux-srv`, which calls a
Whisper backend and returns text. The existing per-pane plumbing
(`getVoiceSession()` singleton, `PaneVoiceHandle`, `MicButton`, the red recording
indicator, "Speak to <agent>" ghost text) is **reused unchanged** — only the
engine behind `getVoiceSession()` swaps.

## 2. Architecture

```
 renderer (pane)                         agentmux-srv                    Whisper
 ───────────────                         ────────────                    ───────
 getUserMedia(audio)                     POST /api/v1/voice/transcribe
   → MediaRecorder ──(webm/opus blob)──▶  (authed, raw audio body)  ──▶  Groq /
   → VAD: cut on silence                  SttBackend::transcribe()        OpenAI /
   ◀──────────── { "text": "..." } ◀──    returns transcript              whisper.cpp
   → PaneVoiceHandle.appendFinal(text)
```

- **No streaming** (no Whisper variant streams): each silence-bounded utterance
  is one request/response. `setInterim` degrades to a "…transcribing" spinner.
- **Key stays server-side.** The renderer never sees the API key; it only POSTs
  audio to the local srv, which holds the key. This is why capture goes through
  srv rather than the renderer calling Groq directly.
- **CEF mic grant (#1602) is the prerequisite** — `getUserMedia({audio:true})`
  only works because of that handler.

## 3. Server: `SttBackend` trait + endpoint

New module `agentmux-srv/src/backend/voice/` (or `server/voice.rs` for the route).

```rust
#[async_trait] // or hand-rolled async fn
pub trait SttBackend: Send + Sync {
    /// Transcribe one audio clip (webm/opus/mp3/wav bytes) → text.
    async fn transcribe(&self, audio: Bytes, mime: &str, lang: Option<&str>)
        -> Result<String, SttError>;
    fn id(&self) -> &'static str;
}
```

**Endpoint** (axum, in `authed_routes`):
```
POST /api/v1/voice/transcribe?mime=audio/webm&lang=en
body: raw audio bytes (axum::body::Bytes)
→ 200 { "text": "..." }   |   501 { "error": "no STT backend configured" }
```
Mirrors the existing `handle_*` pattern: `State(AppState)`, `Bytes` body (same
extractor `service.rs` uses), `Json<Value>` response. Auth-gated like the other
`/api/v1/*` routes.

### 3.1 Backends
- **`GroqBackend`** (PR 1) — `reqwest` multipart POST to
  `https://api.groq.com/openai/v1/audio/transcriptions`, model
  `whisper-large-v3-turbo`. ~$0.0007/min, ~216× real-time. `reqwest` is already
  a dependency.
- **`OpenAiBackend`** (later) — same OpenAI-compatible shape, `gpt-4o-transcribe`.
- **`LocalWhisperBackend`** (PR 2, shipped) — offline whisper.cpp via a local
  **`whisper-cli` subprocess** (not in-process whisper-rs — avoids a bindgen /
  libclang / C++ build in the sidecar). The renderer sends **16 kHz mono WAV**
  for this engine (whisper-cli reads WAV natively, no ffmpeg), captured via
  Web-Audio PCM (`AudioContext({sampleRate:16000})` + `ScriptProcessor`). Both
  the CLI binary is user-provided (`voice:whisperCliPath` /
  `AGENTMUX_WHISPER_CLI`); the **GGML model auto-downloads on first use**
  (PR-3 — default `base.en`, configurable via `voice:whisperModel`, to
  `<config>/whisper-models/`, serialized by a global lock with a 600s cap and
  temp→rename so partial downloads never look valid). An explicit
  `voice:whisperModelPath` overrides and skips the download. Missing binary →
  501. **Bundled CLI binary** (so it's fully zero-config) remains a follow-up —
  per-platform binary fetch is fragile. Fully offline — audio never leaves the
  machine. ~0 MB ship.

### 3.2 Backend selection + key (server-side)
Resolved once at request time from, in order:
1. env `AGENTMUX_GROQ_API_KEY` (and future `AGENTMUX_OPENAI_API_KEY`)
2. settings.json key `voice:groqApiKey` (read from `get_wave_config_dir()/settings.json`)

`voice:engine` setting selects the backend (`groq` | `openai` | `whisper-local`;
default `whisper-local` once PR 2 lands, `groq` in PR 1). If no key/engine is
configured, the endpoint returns `501` and the frontend shows the
`service-not-allowed`-style "not configured" guidance (reuses #1603's toast).

## 4. Frontend: capture engine

New `frontend/app/hook/voice/whisperEngine.ts`, selected by `getVoiceSession()`
based on the `voice:engine` setting. Implements the same `VoiceSession` shape
(`isListening`, `currentTargetId`, `lastError`, `toggleListening`, `registerPane`).

- `getUserMedia({audio:true})` → `MediaRecorder` (webm/opus).
- **VAD:** a Web Audio `AnalyserNode` tracks RMS; on speech→silence (~800 ms)
  or a max-segment timer (~10 s), `recorder.stop()` finalizes a valid webm blob,
  POST it to `/api/v1/voice/transcribe`, then `start()` a fresh recorder.
- On 200 → `activeHandle.appendFinal(text + " ")`. While a segment is in flight
  → `activeHandle.setInterim("…")` (spinner; terminal pane no-ops as today).
- Errors map to the existing `lastError` codes so #1603's UX applies:
  `getUserMedia` `NotAllowedError`→`not-allowed`, `NotFoundError`→`audio-capture`,
  endpoint `501`/network → `service-not-allowed`.

## 5. Gate the dead Web Speech engine (§4d)

`getVoiceSession()` picks the engine from `voice:engine`:
- `webspeech` only if explicitly set AND `webkitSpeechRecognition` exists (escape
  hatch for non-CEF/dev browsers).
- otherwise the Whisper engine.
In CEF, default to the Whisper engine so the mic never presents a working-looking
button backed by a recognizer that can't run.

## 6. Security
- API key never enters the renderer (server-side resolve only).
- Endpoint is auth-gated (`authed_routes`). Mic capture is gated to the trusted
  app origin by the CEF handler (#1602) — browser panes can't capture.
- Audio is sent to the configured provider (Groq/OpenAI) for hosted backends;
  the local whisper.cpp backend (PR 2) keeps audio fully on-device. Document the
  provider egress in settings.

## 7. Phasing
- **PR 1 (this):** `SttBackend` trait + `GroqBackend` + `/api/v1/voice/transcribe`
  + frontend `whisperEngine.ts` (MediaRecorder+VAD) + `voice:engine` setting +
  gate Web Speech. ~0 MB ship, ~600–800 LOC.
- **PR 2:** `LocalWhisperBackend` (whisper-rs) + on-demand model download +
  default engine → `whisper-local`.
- **PR 3 (optional):** Claude post-transcription cleanup pass (agent pane only).

## 8. Verification
- `cargo check -p agentmux-srv` (endpoint + Groq backend).
- `task build:frontend` (capture engine + gating).
- Live: set `voice:groqApiKey`, click mic → OS prompt → speak → text appears in
  composer; confirm key never appears in renderer; confirm browser pane can't
  capture.
