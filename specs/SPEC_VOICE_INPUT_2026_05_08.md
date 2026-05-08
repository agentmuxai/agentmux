# Spec: Voice Input for Agent Pane

**Date:** 2026-05-08  
**Status:** Draft  
**Area:** `frontend/app/view/agent/`, `frontend/app/hook/`

---

## Problem

Users want to dictate messages across multiple agent panes rapidly — click a pane, speak, the transcript appears in that pane's input, move to the next pane, speak again. Typing between separate AI conversations is a bottleneck; voice removes it.

---

## UX Model

```
┌─────────────────────┐   ┌─────────────────────┐
│  Agent Pane A       │   │  Agent Pane B       │
│  ─────────────────  │   │  ─────────────────  │
│  [conversation...]  │   │  [conversation...]  │
│                     │   │                     │
│  ┌───────────────┐  │   │  ┌───────────────┐  │
│  │ "fix the bug" │  │   │  │ |             │  │
│  └───────────────┘  │   │  └───────────────┘  │
│  [🎤 listening]     │   │                     │
└─────────────────────┘   └─────────────────────┘
   user is here                not active
```

**Core behaviours:**

1. **Global mic toggle** — one Ctrl+Shift+M hotkey (or a mic button in the title bar) starts/stops the voice session. The session is global, not per-pane.
2. **Pane-following** — transcript is injected into whichever pane the user last clicked (focused). Switching panes while the mic is on immediately redirects future transcript to the new pane.
3. **Interim display** — partial results appear in the textarea in italic or a distinct style, replaced by the final result when the sentence ends.
4. **No auto-send** — transcript accumulates in the textarea; the user presses Enter to send, exactly as if they had typed it. Voice is an input method, not an autonomous actor.
5. **Coexistence** — the user can type additional text while the mic is on; voice appends after the current cursor position.

---

## Option Analysis

### Option 1 — Web Speech API (Chromium-native)

**What it is:** `webkitSpeechRecognition` / `SpeechRecognition`, built into every Chromium build. No install, no API key.

**How it works:**
- Browser captures mic audio, streams to Google's Speech Recognition servers, returns interim + final results via events (`onresult`, `onspeechend`).
- Continuous mode keeps the session alive between pauses.

**Integration point:** Pure frontend hook (`useVoiceInput.ts`). No backend or sidecar changes. Hooks into `AgentFooter.tsx` textarea ref.

**CEF compatibility:** Fully supported. CEF inherits all Chromium Web APIs. Permission prompt appears once; CEF persists it. On Windows CEF uses the standard browser permission flow.

**Latency:** ~200–500 ms for interim results. Final results within ~1 s of a pause.

**Cost:** Free. Google's infrastructure, silently.

**Privacy:** Audio is sent to Google's speech servers. Acceptable for most users; not for air-gapped or privacy-sensitive deployments.

**Language support:** 100+ locales via `lang` property.

**Limitations:**
- Requires internet (no offline fallback).
- Google controls the recognition quality and rate limits.
- No custom vocabulary / domain-specific terms.

**Verdict:** Best choice for Phase 1. Zero friction, streaming partial results, no API key, already works in CEF.

---

### Option 2 — OpenAI Whisper API

**What it is:** OpenAI's `/v1/audio/transcriptions` REST endpoint. Accepts an audio file, returns a text transcript.

**How it works:**
- Frontend records audio via `MediaRecorder` (WebM/Opus).
- On each pause (voice activity detection), sends the chunk to agentmux-srv via existing RPC.
- agentmux-srv POSTs to `api.openai.com/v1/audio/transcriptions` using the stored OpenAI API key.
- Result is returned via WPS event or RPC response and injected into the focused pane.

**Latency:** 1–3 s per chunk (non-streaming, per-file API). Noticeably laggier than Web Speech API for real-time use.

**Cost:** $0.006/minute (~$0.36/hour of voice input).

**Privacy:** Audio goes to OpenAI. No worse than using OpenAI's chat API.

**Accuracy:** Excellent — Whisper large-v3 is state-of-the-art for most languages.

**Requirements:** OpenAI API key; agentmux-srv HTTP client to proxy the request.

**Verdict:** Good for Phase 2 as a high-accuracy option for users who already have OpenAI credentials. Not ideal for real-time use due to per-chunk latency.

---

### Option 3 — Deepgram Streaming API

**What it is:** Deepgram's `wss://api.deepgram.com/v1/listen` WebSocket. Accepts raw PCM audio chunks, emits word-level transcripts in real time.

**How it works:**
- Frontend opens a WebSocket to agentmux-srv's local WS endpoint.
- agentmux-srv opens a WebSocket to Deepgram's API (proxies audio chunks from the frontend, relays transcript events back).
- Transcripts arrive as JSON events with `is_final` flag for interim vs. committed words.

**Latency:** < 300 ms for interim results — closest to the Web Speech API experience.

**Cost:** $0.0043/minute for the Nova-2 model.

**Accuracy:** Excellent; comparable to Whisper for English. Strong multi-language support.

**Privacy:** Audio goes to Deepgram's servers.

**Requirements:** Deepgram API key; new WS proxy in agentmux-srv; MediaRecorder in frontend.

**Verdict:** Best cloud streaming option. Viable for Phase 2 alongside Whisper, especially for non-English languages.

---

### Option 4 — Local Whisper (whisper.cpp sidecar)

**What it is:** Run [whisper.cpp](https://github.com/ggerganov/whisper.cpp) as a local subprocess inside agentmux-srv. No network, no API key, offline-capable.

**How it works:**
- Frontend records audio via `MediaRecorder` (WebM/Opus → WAV decode).
- Sends audio chunks to agentmux-srv via RPC.
- agentmux-srv pipes chunks to the `whisper-cli` subprocess.
- whisper.cpp emits word-level timestamps and text as JSON to stdout.
- Results relayed back via WPS event to the focused pane.

**Model sizes:**

| Model | Size | WER (en) | Speed (RTF) |
|-------|------|-----------|-------------|
| tiny | 39 MB | ~5% | ~10x real-time |
| base | 74 MB | ~4% | ~7x real-time |
| small | 244 MB | ~3% | ~3x real-time |
| medium | 769 MB | ~2.5% | ~1.5x real-time |
| large-v3 | 1.5 GB | ~2% | ~0.7x real-time |

**Latency:** With the `small` model, ~300–600 ms per utterance on a modern CPU. GPU acceleration (CUDA/Metal) reduces to ~100 ms.

**Cost:** One-time model download. No per-use cost.

**Privacy:** Fully local. No audio leaves the machine.

**Requirements:** whisper.cpp binary distributed or downloaded on demand; model file (~74–244 MB for practical sizes); agentmux-srv subprocess management.

**Verdict:** Best for Phase 3 / privacy-focused users. Meaningful download UX required. whisper.cpp already supports Windows (MSVC), macOS, and Linux.

---

### Option 5 — Claude Multimodal (Audio → Text via Claude API)

**What it is:** Send audio to Claude as a file and ask it to transcribe.

**Verdict:** Not suitable. Claude's API is not designed for real-time STT. Round-trip latency would be 3–10 s. Wastes input tokens. No interim results. Ruled out.

---

## Recommendation: Phased Rollout

| Phase | Approach | Effort | Latency | Cost | Privacy |
|-------|----------|--------|---------|------|---------|
| 1 | Web Speech API | Low — frontend only | ~300 ms | Free | Google |
| 2 | Whisper API / Deepgram | Medium — sidecar proxy | ~1–3 s / ~300 ms | $0.006/min | OpenAI/DG |
| 3 | Local whisper.cpp | High — binary + model | ~300–600 ms | Free | Fully local |

**Phase 1 is what ships first.** It requires zero infrastructure changes and covers the vast majority of users. Phases 2 and 3 are provider-select additions in the settings pane.

---

## Phase 1 Architecture (Web Speech API)

### New files

```
frontend/app/hook/useVoiceInput.ts          — global voice session singleton
frontend/app/view/agent/components/MicButton.tsx  — icon button for the footer
```

### Modified files

```
frontend/app/view/agent/components/AgentFooter.tsx  — wire MicButton + transcript injection
frontend/app/view/agent/agent-view.tsx              — propagate focused pane to voice hook
frontend/app/app-init.ts                            — install global Ctrl+Shift+M handler
```

---

### `useVoiceInput.ts` — Global singleton hook

```ts
// Singleton: one SpeechRecognition instance shared across all panes.
// The "active pane" is tracked via a signal updated on pane focus.

interface VoiceInputHandle {
    isListening: () => boolean;
    interimText: () => string;
    toggleListening: () => void;
    setActivePaneRef: (ref: { append: (text: string) => void } | null) => void;
}
```

**State machine:**

```
idle
  │ toggleListening()
  ▼
listening
  │ onresult (interim) → inject interimText into focused pane textarea
  │ onresult (final)   → commit final text, clear interim
  │ onend              → auto-restart (continuous mode)
  │ toggleListening()
  ▼
idle
```

**Key implementation details:**

```ts
const recognition = new (window.SpeechRecognition ?? window.webkitSpeechRecognition)();
recognition.continuous = true;
recognition.interimResults = true;
recognition.lang = navigator.language;  // user's OS locale

recognition.onresult = (event) => {
    let interim = "";
    let final = "";
    for (const result of Array.from(event.results).slice(event.resultIndex)) {
        if (result.isFinal) final += result[0].transcript;
        else interim += result[0].transcript;
    }
    if (final) activePaneRef?.appendFinal(final);
    setInterimText(interim);
};

recognition.onend = () => {
    // Auto-restart: Web Speech API stops after a pause on some browsers.
    if (isListening()) recognition.start();
};
```

**Pane focus tracking:**
- Each `AgentFooter` calls `setActivePaneRef` with a `{ appendFinal, setInterim }` handle on mount and on every focus event.
- The handle writes into the textarea's current value via the `textareaRef`.
- On pane blur, the handle is not cleared (voice continues mid-sentence); it is replaced when another pane gains focus.

---

### `MicButton.tsx` — Mic indicator in footer

Small icon button placed left of the textarea. Uses existing `IconButton` pattern.

```
┌────────────────────────────────────────────────────────┐
│  🎤  │ [transcript appears here as you speak...]       │
└────────────────────────────────────────────────────────┘
```

States:
- **Idle:** Mic icon, muted color. Click to start.
- **Listening:** Animated pulse ring around the mic icon (CSS keyframe), primary color.
- **Interim text present:** Mic icon + subtle italic text overlay in textarea.

The mic button is always visible in the footer (does not appear/disappear) so users know voice input is available.

---

### Textarea interim text display

Interim results are appended to the textarea's value in a way that is clearly distinguishable and replaceable:

```ts
// When interim arrives:
textareaRef.value = committedText + " " + interimTranscript;
// Style the interim portion: not trivial in a plain textarea.
// Simpler: suffix with a cursor-like marker and rely on italics via
// a sibling ghost element overlaid on the textarea (same approach as
// autocomplete overlays in modern IDEs).
```

Simplest workable approach: a `<div>` overlay (same font, same size, transparent background) renders `committedText` as invisible and `interimText` as dimmed/italic. When the final result arrives, `committedText` is updated and the overlay is cleared.

---

### Keyboard shortcut

**Ctrl+Shift+M** — global, installed at app-init level. Toggles the voice session regardless of which element has focus.

Works even when the user's cursor is in a terminal pane (voice output still targets the last focused agent pane, not the terminal).

---

### Permission flow

On first toggle, the browser shows the standard "Allow microphone" prompt (CEF inherits Chromium's permission UI). The permission is persisted by CEF's user-data profile (per version, as with all CEF state).

If the user denies permission, `recognition.onerror` fires with `{ error: "not-allowed" }`. Show a toast: "Microphone access denied — allow it in your OS sound settings."

---

## Phase 2 Architecture (Provider-based STT)

Add a `speechProvider` field to the settings schema:

```ts
type SpeechProvider = 
    | { type: "web-speech" }                         // Phase 1 (default)
    | { type: "openai-whisper"; apiKey?: string }     // uses stored OpenAI key
    | { type: "deepgram"; apiKey: string }
    | { type: "local-whisper"; model: "tiny" | "base" | "small" | "medium" }
```

The `useVoiceInput` hook reads `speechProvider` from settings and routes audio accordingly.

For Whisper/Deepgram: `MediaRecorder` captures audio → sends chunks to agentmux-srv → srv proxies to API → WPS event delivers transcript.

For local Whisper: same IPC path but agentmux-srv calls whisper.cpp subprocess.

---

## Edge Cases

| Scenario | Behaviour |
|----------|-----------|
| User switches pane mid-sentence | Final result goes to the pane active when the sentence ends |
| Agent is running (streaming response) | Transcript still injected into textarea; send is blocked until stream ends (existing behaviour) |
| Two agent panes in the same window | Last focused wins |
| User opens a terminal pane | Terminal pane is not an agent pane; does not register a voice handle; previous agent pane keeps the target |
| Mic permission denied | Toast + mic button shows error state; toggle does nothing until page reload |
| No SpeechRecognition API in browser | Mic button is hidden; falls through to Phase 2 provider if configured |
| Multiple AgentMux windows | Each window's voice session is independent; Ctrl+Shift+M affects the focused window |

---

## Non-Goals

- **Voice commands** (e.g., "stop agent", "scroll to top") — voice is input method only, not a command interface.
- **Text-to-speech / agent speaking back** — out of scope for this spec.
- **Wake word** ("hey mux") — not planned; hotkey is sufficient and less invasive.
- **Multi-speaker diarization** — not relevant for single-user desktop.
- **Recording history** — voice input is ephemeral; nothing is stored.

---

## Implementation Order

1. `useVoiceInput.ts` — Web Speech API session, pane handle registry, `isListening` signal.
2. `MicButton.tsx` — icon button with pulse animation.
3. `AgentFooter.tsx` — mount/focus events to register pane handle; wire interim text overlay.
4. `agent-view.tsx` — pass `onFocus` callback to footer.
5. `app-init.ts` — global Ctrl+Shift+M keybinding.
6. Manual test: open two panes, toggle mic, alternate speaking into each.

**Estimated effort:** Phase 1 is ~2–3 days frontend-only work. No Rust changes required.
