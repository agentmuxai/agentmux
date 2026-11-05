// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whisper capture-and-send voice engine.
 *
 * The Web Speech API can't transcribe in CEF (closed-source Google service,
 * Chrome-build-bound), so this engine captures mic audio, cuts it into
 * silence-bounded utterances (a small Web-Audio VAD), and POSTs each clip to
 * `agentmux-srv` (`/api/v1/voice/transcribe`), which calls Whisper and returns
 * text. The API key / model path stays server-side.
 *
 * Two capture modes, chosen by the `voice:engine` setting so each backend gets
 * audio it can decode without a server-side decoder:
 *   - **groq** (default): MediaRecorder → webm/opus (Groq decodes server-side).
 *   - **whisper-local**: Web-Audio PCM → 16 kHz mono WAV (whisper.cpp's
 *     `whisper-cli` reads WAV natively; no ffmpeg needed).
 *
 * Implements the same `VoiceSession` shape as the Web Speech engine so
 * `getVoiceSession()` swaps engines with the rest of the per-pane plumbing
 * (MicButton, red indicator, "Speak to <agent>" ghost text) unchanged.
 *
 * Spec: docs/specs/SPEC_VOICE_STT_ENGINE_2026_06_20.md · tracking #1591.
 */

import { createSignalAtom, type SignalAtom } from "@/util/util";
import { authHeaders } from "@/app/store/auth-headers";
import { getWebServerEndpoint } from "@/util/endpoints";
import { getSettingsKeyAtom } from "@/app/store/global";
import { computeRms } from "./audioLevel";
import type { PaneVoiceHandle, VoiceSession } from "./useVoiceInput";

// VAD tuning. RMS below SILENCE_RMS for >= SILENCE_MS after speech ends an
// utterance; a hard cap keeps any single clip small (Whisper isn't streaming,
// so shorter clips = lower latency). LEVEL_POLL_MS samples the analyser in webm
// mode — a periodic read is inherent to Web-Audio level metering, not a hack.
const SILENCE_RMS = 0.012;
const SILENCE_MS = 800;
const MAX_SEGMENT_MS = 12_000;
const MIN_SEGMENT_MS = 350; // drop blips with no real speech
const LEVEL_POLL_MS = 100;
const WAV_SAMPLE_RATE = 16_000; // whisper.cpp requires 16 kHz mono

/** Pick the best MediaRecorder mime the runtime supports (opus preferred). */
export function pickMime(): string {
    const candidates = [
        "audio/webm;codecs=opus",
        "audio/webm",
        "audio/ogg;codecs=opus",
        "audio/mp4",
    ];
    for (const m of candidates) {
        if (typeof MediaRecorder !== "undefined" && MediaRecorder.isTypeSupported(m)) {
            return m;
        }
    }
    return "audio/webm";
}

/** Encode mono Float32 PCM as a 16-bit WAV blob at the given sample rate. */
function encodeWav(samples: Float32Array, sampleRate: number): Blob {
    const buffer = new ArrayBuffer(44 + samples.length * 2);
    const view = new DataView(buffer);
    const writeStr = (off: number, s: string) => {
        for (let i = 0; i < s.length; i++) view.setUint8(off + i, s.charCodeAt(i));
    };
    writeStr(0, "RIFF");
    view.setUint32(4, 36 + samples.length * 2, true);
    writeStr(8, "WAVE");
    writeStr(12, "fmt ");
    view.setUint32(16, 16, true); // PCM fmt chunk size
    view.setUint16(20, 1, true); // PCM
    view.setUint16(22, 1, true); // mono
    view.setUint32(24, sampleRate, true);
    view.setUint32(28, sampleRate * 2, true); // byte rate (mono, 16-bit)
    view.setUint16(32, 2, true); // block align
    view.setUint16(34, 16, true); // bits per sample
    writeStr(36, "data");
    view.setUint32(40, samples.length * 2, true);
    let off = 44;
    for (let i = 0; i < samples.length; i++) {
        const s = Math.max(-1, Math.min(1, samples[i]));
        view.setInt16(off, s < 0 ? s * 0x8000 : s * 0x7fff, true);
        off += 2;
    }
    return new Blob([view], { type: "audio/wav" });
}

/** What one voice session and the functions that serve it share. */
interface WhisperState {
    isListening: SignalAtom<boolean>;
    currentTargetId: SignalAtom<string | null>;
    lastError: SignalAtom<string | null>;
    lastErrorDetail: SignalAtom<string | null>;
    isLocal: boolean;
    activeHandle: PaneVoiceHandle | null;
    stream: MediaStream | null;
    audioCtx: AudioContext | null;
    mime: string;

    // webm mode
    recorder: MediaRecorder | null;
    analyser: AnalyserNode | null;
    levelTimer: number | null;
    chunks: Blob[];

    // wav mode
    processor: ScriptProcessorNode | null;
    pcm: Float32Array[];

    // shared VAD state
    sawSpeech: boolean;
    silenceStart: number;
    segmentStart: number;

    // Serialize transcription so utterances are applied in spoken order.
    // Capture keeps running concurrently; only the network call + appendFinal
    // are chained — Groq/whisper latency varies, so POSTing concurrently could
    // resolve out of order and scramble the composer text (reagent P1 #1623).
    postChain: Promise<void>;
}

function enqueuePost(s: WhisperState, blob: Blob) {
    s.postChain = s.postChain.then(() => postSegment(s, blob)).catch(() => {});
}

function fail(s: WhisperState, code: string) {
    s.lastError._set(code);
    window.dispatchEvent(new CustomEvent("voice-input-error", { detail: code }));
    void stopCapture(s);
}

async function postSegment(s: WhisperState, blob: Blob) {
    if (blob.size === 0) return;
    const handle = s.activeHandle;
    if (!handle) return;
    handle.setInterim("…"); // non-streaming: spinner-ish placeholder
    try {
        const base = getWebServerEndpoint();
        const url = `${base}/api/v1/voice/transcribe?mime=${encodeURIComponent(s.mime)}&lang=${encodeURIComponent(
            (navigator.language || "").split("-")[0] || "",
        )}`;
        const resp = await fetch(url, {
            method: "POST",
            headers: authHeaders({ "Content-Type": s.mime }),
            body: blob,
        });
        handle.setInterim("");
        if (!resp.ok) {
            // Thread the server's actual error body through as a detail —
            // additive: existing coarse `lastError` categories used by
            // MicButton.tsx's tooltip stay unchanged, this only adds a
            // detail string for surfaces that want it (the Settings ->
            // Recording test-mic panel).
            s.lastErrorDetail._set(await readErrorDetail(resp));
            if (resp.status === 501) {
                fail(s, "service-not-allowed"); // backend not configured
            } else {
                s.lastError._set("service-not-allowed");
                window.dispatchEvent(new CustomEvent("voice-input-error", { detail: "service-not-allowed" }));
            }
            return;
        }
        s.lastErrorDetail._set(null);
        const data = (await resp.json()) as { text?: string };
        const text = (data.text || "").trim();
        if (text) handle.appendFinal(text);
    } catch (e) {
        handle.setInterim("");
        s.lastError._set("service-not-allowed");
        s.lastErrorDetail._set(e instanceof Error ? e.message : null);
        window.dispatchEvent(new CustomEvent("voice-input-error", { detail: "service-not-allowed" }));
    }
}

/** Read `{ "error": "..." }` from a non-OK transcribe response, best-effort. */
async function readErrorDetail(resp: Response): Promise<string | null> {
    try {
        const data = (await resp.json()) as { error?: string };
        return data.error ?? null;
    } catch {
        return null;
    }
}

// VAD applied to a fresh RMS reading; returns true when the utterance should
// be cut. Shared by both modes.
function vadShouldCut(s: WhisperState, rms: number): boolean {
    const now = Date.now();
    if (rms >= SILENCE_RMS) {
        s.sawSpeech = true;
        s.silenceStart = 0;
    } else if (s.sawSpeech) {
        if (s.silenceStart === 0) s.silenceStart = now;
        else if (now - s.silenceStart >= SILENCE_MS) return true;
    }
    // Hard cap fires regardless of sawSpeech so the WAV-mode PCM buffer
    // can't grow unbounded under sustained sub-threshold input — a
    // speechless cut just discards + resets (postSegment/cut are gated on
    // sawSpeech). webm mode is bounded by MediaRecorder internally.
    return now - s.segmentStart >= MAX_SEGMENT_MS;
}

function resetSegment(s: WhisperState) {
    s.sawSpeech = false;
    s.silenceStart = 0;
    s.segmentStart = Date.now();
}

// ── webm mode (MediaRecorder + AnalyserNode VAD) ─────────────────────────

function startRecorder(s: WhisperState) {
    if (!s.stream) return;
    s.chunks = [];
    resetSegment(s);
    s.recorder = new MediaRecorder(s.stream, { mimeType: s.mime });
    s.recorder.ondataavailable = (e) => {
        if (e.data && e.data.size > 0) s.chunks.push(e.data);
    };
    s.recorder.onstop = () => {
        const dur = Date.now() - s.segmentStart;
        const blob = new Blob(s.chunks, { type: s.mime });
        s.chunks = [];
        if (s.sawSpeech && dur >= MIN_SEGMENT_MS) enqueuePost(s, blob);
        if (s.isListening()) startRecorder(s);
    };
    s.recorder.start();
}

function pollLevel(s: WhisperState) {
    if (!s.analyser) return;
    if (vadShouldCut(s, computeRms(s.analyser))) {
        if (s.recorder && s.recorder.state !== "inactive") s.recorder.stop();
    }
}

// ── wav mode (ScriptProcessor PCM @ 16 kHz) ──────────────────────────────

function cutWavSegment(s: WhisperState) {
    const dur = Date.now() - s.segmentStart;
    const total = s.pcm.reduce((n, c) => n + c.length, 0);
    const merged = new Float32Array(total);
    let off = 0;
    for (const c of s.pcm) {
        merged.set(c, off);
        off += c.length;
    }
    s.pcm = [];
    if (s.sawSpeech && dur >= MIN_SEGMENT_MS && merged.length > 0) {
        enqueuePost(s, encodeWav(merged, WAV_SAMPLE_RATE));
    }
    resetSegment(s);
}

function onAudioProcess(s: WhisperState, e: AudioProcessingEvent) {
    const input = e.inputBuffer.getChannelData(0);
    // Copy (the buffer is reused by the engine).
    s.pcm.push(new Float32Array(input));
    let sum = 0;
    for (let i = 0; i < input.length; i++) sum += input[i] * input[i];
    if (vadShouldCut(s, Math.sqrt(sum / input.length))) cutWavSegment(s);
    // Leave outputBuffer silent (don't write it) → no mic echo to speakers.
}

// ── lifecycle ────────────────────────────────────────────────────────────

async function stopCapture(s: WhisperState) {
    s.isListening._set(false);
    s.currentTargetId._set(null);
    if (s.levelTimer != null) {
        clearInterval(s.levelTimer);
        s.levelTimer = null;
    }
    if (s.recorder && s.recorder.state !== "inactive") {
        try {
            s.recorder.onstop = null as any; // don't restart on deliberate stop
            s.recorder.stop();
        } catch {
            /* ignore */
        }
    }
    s.recorder = null;
    if (s.processor) {
        try {
            s.processor.onaudioprocess = null as any;
            s.processor.disconnect();
        } catch {
            /* ignore */
        }
        s.processor = null;
    }
    s.analyser = null;
    if (s.audioCtx) {
        try { await s.audioCtx.close(); } catch { /* ignore */ }
        s.audioCtx = null;
    }
    if (s.stream) {
        s.stream.getTracks().forEach((t) => t.stop());
        s.stream = null;
    }
    s.activeHandle?.setInterim("");
}

async function startCapture(s: WhisperState) {
    try {
        // voice:inputDeviceId (Settings -> Recording device picker):
        // absent/"default" keeps the original unconstrained behavior.
        const deviceId = getSettingsKeyAtom("voice:inputDeviceId")();
        const audioConstraint: boolean | MediaTrackConstraints =
            deviceId && deviceId !== "default" ? { deviceId: { exact: deviceId } } : true;
        s.stream = await navigator.mediaDevices.getUserMedia({ audio: audioConstraint });
    } catch (e: any) {
        const name = e?.name || "";
        if (name === "NotAllowedError" || name === "SecurityError") fail(s, "not-allowed");
        else if (name === "NotFoundError" || name === "OverconstrainedError") fail(s, "audio-capture");
        else fail(s, "service-not-allowed");
        return;
    }
    s.lastError._set(null);
    s.isListening._set(true);

    // Building the audio graph can throw — e.g. a runtime that rejects the
    // forced 16 kHz AudioContext rate. Catch it so the mic stream is closed
    // and isListening doesn't get stuck true (fail → stopCapture).
    try {
        if (s.isLocal) {
            s.mime = "audio/wav";
            // Force 16 kHz so captured PCM is whisper-ready (no resampling).
            s.audioCtx = new AudioContext({ sampleRate: WAV_SAMPLE_RATE });
            const src = s.audioCtx.createMediaStreamSource(s.stream);
            s.processor = s.audioCtx.createScriptProcessor(4096, 1, 1);
            s.processor.onaudioprocess = (e) => onAudioProcess(s, e);
            src.connect(s.processor);
            s.processor.connect(s.audioCtx.destination); // required for onaudioprocess to fire
            s.pcm = [];
            resetSegment(s);
        } else {
            s.mime = pickMime();
            s.audioCtx = new AudioContext();
            const src = s.audioCtx.createMediaStreamSource(s.stream);
            s.analyser = s.audioCtx.createAnalyser();
            s.analyser.fftSize = 2048;
            src.connect(s.analyser);
            startRecorder(s);
            s.levelTimer = window.setInterval(() => pollLevel(s), LEVEL_POLL_MS);
        }
    } catch {
        fail(s, "service-not-allowed");
    }
}

function toggleListening(s: WhisperState) {
    if (s.isListening()) void stopCapture(s);
    else void startCapture(s);
}

export function createWhisperVoiceSession(): VoiceSession {
    const isListening = createSignalAtom(false);
    const currentTargetId = createSignalAtom<string | null>(null);
    const lastError = createSignalAtom<string | null>(null);
    const lastErrorDetail = createSignalAtom<string | null>(null);

    // Capture mode is fixed for the session: whisper-local → WAV, else webm.
    const isLocal = getSettingsKeyAtom("voice:engine")() === "whisper-local";

    // Both modes need getUserMedia + AudioContext; the webm (groq) path also
    // needs MediaRecorder. Gate on exactly what the chosen mode uses so a
    // runtime missing a primitive degrades gracefully instead of throwing.
    const available =
        typeof navigator !== "undefined" &&
        !!navigator.mediaDevices?.getUserMedia &&
        typeof AudioContext !== "undefined" &&
        (isLocal || typeof MediaRecorder !== "undefined");

    if (!available) {
        return {
            isListening,
            currentTargetId,
            lastError,
            lastErrorDetail,
            isAvailable: () => false,
            toggleListening: () => {},
            registerPane: () => {},
        };
    }

    const s: WhisperState = {
        isListening,
        currentTargetId,
        lastError,
        lastErrorDetail,
        isLocal,
        activeHandle: null,
        stream: null,
        audioCtx: null,
        mime: "audio/webm",
        recorder: null,
        analyser: null,
        levelTimer: null,
        chunks: [],
        processor: null,
        pcm: [],
        sawSpeech: false,
        silenceStart: 0,
        segmentStart: 0,
        postChain: Promise.resolve(),
    };

    return {
        isListening,
        currentTargetId,
        lastError,
        lastErrorDetail,
        isAvailable: () => true,
        toggleListening: () => toggleListening(s),
        registerPane: (blockId, handle) => {
            s.activeHandle = handle;
            currentTargetId._set(blockId);
        },
    };
}
