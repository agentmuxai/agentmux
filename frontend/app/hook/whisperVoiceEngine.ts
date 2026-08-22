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

import { createSignalAtom } from "@/util/util";
import { getWebServerEndpoint } from "@/util/endpoints";
import { getApi } from "@/app/store/app-api";
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

    let activeHandle: PaneVoiceHandle | null = null;
    let stream: MediaStream | null = null;
    let audioCtx: AudioContext | null = null;
    let mime = "audio/webm";

    // webm mode
    let recorder: MediaRecorder | null = null;
    let analyser: AnalyserNode | null = null;
    let levelTimer: number | null = null;
    let chunks: Blob[] = [];

    // wav mode
    let processor: ScriptProcessorNode | null = null;
    let pcm: Float32Array[] = [];

    // shared VAD state
    let sawSpeech = false;
    let silenceStart = 0;
    let segmentStart = 0;

    // Serialize transcription so utterances are applied in spoken order.
    // Capture keeps running concurrently; only the network call + appendFinal
    // are chained — Groq/whisper latency varies, so POSTing concurrently could
    // resolve out of order and scramble the composer text (reagent P1 #1623).
    let postChain: Promise<void> = Promise.resolve();
    const enqueuePost = (blob: Blob) => {
        postChain = postChain.then(() => postSegment(blob)).catch(() => {});
    };

    const fail = (code: string) => {
        lastError._set(code);
        window.dispatchEvent(new CustomEvent("voice-input-error", { detail: code }));
        void stopCapture();
    };

    const postSegment = async (blob: Blob) => {
        if (blob.size === 0) return;
        const handle = activeHandle;
        if (!handle) return;
        handle.setInterim("…"); // non-streaming: spinner-ish placeholder
        try {
            const base = getWebServerEndpoint();
            const url = `${base}/api/v1/voice/transcribe?mime=${encodeURIComponent(mime)}&lang=${encodeURIComponent(
                (navigator.language || "").split("-")[0] || "",
            )}`;
            const resp = await fetch(url, {
                method: "POST",
                headers: { "X-AuthKey": getApi()?.getAuthKey?.() ?? "", "Content-Type": mime },
                body: blob,
            });
            handle.setInterim("");
            if (!resp.ok) {
                // Thread the server's actual error body through as a detail —
                // additive: existing coarse `lastError` categories used by
                // MicButton.tsx's tooltip stay unchanged, this only adds a
                // detail string for surfaces that want it (the Settings ->
                // Recording test-mic panel).
                lastErrorDetail._set(await readErrorDetail(resp));
                if (resp.status === 501) {
                    fail("service-not-allowed"); // backend not configured
                } else {
                    lastError._set("service-not-allowed");
                    window.dispatchEvent(new CustomEvent("voice-input-error", { detail: "service-not-allowed" }));
                }
                return;
            }
            lastErrorDetail._set(null);
            const data = (await resp.json()) as { text?: string };
            const text = (data.text || "").trim();
            if (text) handle.appendFinal(text);
        } catch (e) {
            handle.setInterim("");
            lastError._set("service-not-allowed");
            lastErrorDetail._set(e instanceof Error ? e.message : null);
            window.dispatchEvent(new CustomEvent("voice-input-error", { detail: "service-not-allowed" }));
        }
    };

    /** Read `{ "error": "..." }` from a non-OK transcribe response, best-effort. */
    const readErrorDetail = async (resp: Response): Promise<string | null> => {
        try {
            const data = (await resp.json()) as { error?: string };
            return data.error ?? null;
        } catch {
            return null;
        }
    };

    // VAD applied to a fresh RMS reading; returns true when the utterance should
    // be cut. Shared by both modes.
    const vadShouldCut = (rms: number): boolean => {
        const now = Date.now();
        if (rms >= SILENCE_RMS) {
            sawSpeech = true;
            silenceStart = 0;
        } else if (sawSpeech) {
            if (silenceStart === 0) silenceStart = now;
            else if (now - silenceStart >= SILENCE_MS) return true;
        }
        // Hard cap fires regardless of sawSpeech so the WAV-mode PCM buffer
        // can't grow unbounded under sustained sub-threshold input — a
        // speechless cut just discards + resets (postSegment/cut are gated on
        // sawSpeech). webm mode is bounded by MediaRecorder internally.
        return now - segmentStart >= MAX_SEGMENT_MS;
    };

    const resetSegment = () => {
        sawSpeech = false;
        silenceStart = 0;
        segmentStart = Date.now();
    };

    // ── webm mode (MediaRecorder + AnalyserNode VAD) ─────────────────────────

    const startRecorder = () => {
        if (!stream) return;
        chunks = [];
        resetSegment();
        recorder = new MediaRecorder(stream, { mimeType: mime });
        recorder.ondataavailable = (e) => {
            if (e.data && e.data.size > 0) chunks.push(e.data);
        };
        recorder.onstop = () => {
            const dur = Date.now() - segmentStart;
            const blob = new Blob(chunks, { type: mime });
            chunks = [];
            if (sawSpeech && dur >= MIN_SEGMENT_MS) enqueuePost(blob);
            if (isListening()) startRecorder();
        };
        recorder.start();
    };

    const pollLevel = () => {
        if (!analyser) return;
        if (vadShouldCut(computeRms(analyser))) {
            if (recorder && recorder.state !== "inactive") recorder.stop();
        }
    };

    // ── wav mode (ScriptProcessor PCM @ 16 kHz) ──────────────────────────────

    const cutWavSegment = () => {
        const dur = Date.now() - segmentStart;
        const total = pcm.reduce((n, c) => n + c.length, 0);
        const merged = new Float32Array(total);
        let off = 0;
        for (const c of pcm) {
            merged.set(c, off);
            off += c.length;
        }
        pcm = [];
        if (sawSpeech && dur >= MIN_SEGMENT_MS && merged.length > 0) {
            enqueuePost(encodeWav(merged, WAV_SAMPLE_RATE));
        }
        resetSegment();
    };

    const onAudioProcess = (e: AudioProcessingEvent) => {
        const input = e.inputBuffer.getChannelData(0);
        // Copy (the buffer is reused by the engine).
        pcm.push(new Float32Array(input));
        let sum = 0;
        for (let i = 0; i < input.length; i++) sum += input[i] * input[i];
        if (vadShouldCut(Math.sqrt(sum / input.length))) cutWavSegment();
        // Leave outputBuffer silent (don't write it) → no mic echo to speakers.
    };

    // ── lifecycle ────────────────────────────────────────────────────────────

    const stopCapture = async () => {
        isListening._set(false);
        currentTargetId._set(null);
        if (levelTimer != null) {
            clearInterval(levelTimer);
            levelTimer = null;
        }
        if (recorder && recorder.state !== "inactive") {
            try {
                recorder.onstop = null as any; // don't restart on deliberate stop
                recorder.stop();
            } catch {
                /* ignore */
            }
        }
        recorder = null;
        if (processor) {
            try {
                processor.onaudioprocess = null as any;
                processor.disconnect();
            } catch {
                /* ignore */
            }
            processor = null;
        }
        analyser = null;
        if (audioCtx) {
            try { await audioCtx.close(); } catch { /* ignore */ }
            audioCtx = null;
        }
        if (stream) {
            stream.getTracks().forEach((t) => t.stop());
            stream = null;
        }
        activeHandle?.setInterim("");
    };

    const startCapture = async () => {
        try {
            // voice:inputDeviceId (Settings -> Recording device picker):
            // absent/"default" keeps the original unconstrained behavior.
            const deviceId = getSettingsKeyAtom("voice:inputDeviceId")();
            const audioConstraint: boolean | MediaTrackConstraints =
                deviceId && deviceId !== "default" ? { deviceId: { exact: deviceId } } : true;
            stream = await navigator.mediaDevices.getUserMedia({ audio: audioConstraint });
        } catch (e: any) {
            const name = e?.name || "";
            if (name === "NotAllowedError" || name === "SecurityError") fail("not-allowed");
            else if (name === "NotFoundError" || name === "OverconstrainedError") fail("audio-capture");
            else fail("service-not-allowed");
            return;
        }
        lastError._set(null);
        isListening._set(true);

        // Building the audio graph can throw — e.g. a runtime that rejects the
        // forced 16 kHz AudioContext rate. Catch it so the mic stream is closed
        // and isListening doesn't get stuck true (fail → stopCapture).
        try {
            if (isLocal) {
                mime = "audio/wav";
                // Force 16 kHz so captured PCM is whisper-ready (no resampling).
                audioCtx = new AudioContext({ sampleRate: WAV_SAMPLE_RATE });
                const src = audioCtx.createMediaStreamSource(stream);
                processor = audioCtx.createScriptProcessor(4096, 1, 1);
                processor.onaudioprocess = onAudioProcess;
                src.connect(processor);
                processor.connect(audioCtx.destination); // required for onaudioprocess to fire
                pcm = [];
                resetSegment();
            } else {
                mime = pickMime();
                audioCtx = new AudioContext();
                const src = audioCtx.createMediaStreamSource(stream);
                analyser = audioCtx.createAnalyser();
                analyser.fftSize = 2048;
                src.connect(analyser);
                startRecorder();
                levelTimer = window.setInterval(pollLevel, LEVEL_POLL_MS);
            }
        } catch {
            fail("service-not-allowed");
        }
    };

    const toggleListening = () => {
        if (isListening()) void stopCapture();
        else void startCapture();
    };

    return {
        isListening,
        currentTargetId,
        lastError,
        lastErrorDetail,
        isAvailable: () => true,
        toggleListening,
        registerPane: (blockId, handle) => {
            activeHandle = handle;
            currentTargetId._set(blockId);
        },
    };
}
