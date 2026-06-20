// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whisper capture-and-send voice engine.
 *
 * The Web Speech API can't transcribe in CEF (closed-source Google service,
 * Chrome-build-bound), so this engine captures mic audio with getUserMedia +
 * MediaRecorder, cuts it into silence-bounded utterances (a small Web-Audio
 * VAD), and POSTs each clip to `agentmux-srv` (`/api/v1/voice/transcribe`),
 * which calls Whisper and returns text. The API key stays server-side.
 *
 * Implements the same `VoiceSession` shape as the Web Speech engine so
 * `getVoiceSession()` can pick between them with the rest of the per-pane
 * plumbing (MicButton, red indicator, "Speak to <agent>" ghost text) unchanged.
 *
 * Spec: docs/specs/SPEC_VOICE_STT_ENGINE_2026_06_20.md · tracking #1591.
 */

import { createSignalAtom } from "@/util/util";
import { getWebServerEndpoint } from "@/util/endpoints";
import { getApi } from "@/app/store/app-api";
import type { PaneVoiceHandle, VoiceSession } from "./useVoiceInput";

// VAD tuning. RMS below SILENCE_RMS for >= SILENCE_MS after speech ends an
// utterance; a hard cap keeps any single clip small (Whisper isn't streaming,
// so shorter clips = lower latency). LEVEL_POLL_MS samples the analyser — a
// periodic read is inherent to Web-Audio level metering, not a workaround.
const SILENCE_RMS = 0.012;
const SILENCE_MS = 800;
const MAX_SEGMENT_MS = 12_000;
const MIN_SEGMENT_MS = 350; // drop blips with no real speech
const LEVEL_POLL_MS = 100;

/** Pick the best MediaRecorder mime the runtime supports (opus preferred). */
function pickMime(): string {
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

export function createWhisperVoiceSession(): VoiceSession {
    const isListening = createSignalAtom(false);
    const currentTargetId = createSignalAtom<string | null>(null);
    const lastError = createSignalAtom<string | null>(null);

    const available =
        typeof navigator !== "undefined" &&
        !!navigator.mediaDevices?.getUserMedia &&
        typeof MediaRecorder !== "undefined";

    if (!available) {
        return {
            isListening,
            currentTargetId,
            lastError,
            isAvailable: () => false,
            toggleListening: () => {},
            registerPane: () => {},
        };
    }

    let activeHandle: PaneVoiceHandle | null = null;
    let stream: MediaStream | null = null;
    let recorder: MediaRecorder | null = null;
    let audioCtx: AudioContext | null = null;
    let analyser: AnalyserNode | null = null;
    let levelTimer: number | null = null;
    let chunks: BlobChunk[] = [];
    let mime = "audio/webm";

    // VAD state
    let sawSpeech = false;
    let silenceStart = 0;
    let segmentStart = 0;

    // Serialize transcription so utterances are applied in spoken order.
    // Capture keeps running concurrently (recorder restarts immediately); only
    // the network call + appendFinal are chained — Groq latency varies, so
    // POSTing concurrently could resolve out of order and scramble the composer
    // text during continuous speech (reagent P1 #1623).
    let postChain: Promise<void> = Promise.resolve();
    const enqueuePost = (blob: Blob) => {
        postChain = postChain.then(() => postSegment(blob)).catch(() => {});
    };

    type BlobChunk = Blob;

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
            if (resp.status === 501) {
                fail("service-not-allowed"); // backend not configured
                return;
            }
            if (!resp.ok) {
                // Transient upstream error — surface once, keep the session alive.
                lastError._set("service-not-allowed");
                window.dispatchEvent(new CustomEvent("voice-input-error", { detail: "service-not-allowed" }));
                return;
            }
            const data = (await resp.json()) as { text?: string };
            const text = (data.text || "").trim();
            if (text) handle.appendFinal(text);
        } catch {
            handle.setInterim("");
            // Network/host error — surface as service-unavailable.
            lastError._set("service-not-allowed");
            window.dispatchEvent(new CustomEvent("voice-input-error", { detail: "service-not-allowed" }));
        }
    };

    /** Stop the current recorder; its onstop posts the accumulated clip. */
    const cutSegment = () => {
        if (recorder && recorder.state !== "inactive") {
            recorder.stop(); // → onstop builds blob + posts + restarts (if listening)
        }
    };

    const startRecorder = () => {
        if (!stream) return;
        chunks = [];
        sawSpeech = false;
        silenceStart = 0;
        segmentStart = Date.now();
        recorder = new MediaRecorder(stream, { mimeType: mime });
        recorder.ondataavailable = (e) => {
            if (e.data && e.data.size > 0) chunks.push(e.data);
        };
        recorder.onstop = () => {
            const dur = Date.now() - segmentStart;
            const blob = new Blob(chunks, { type: mime });
            chunks = [];
            // Only transcribe clips that contained speech and weren't blips.
            // Enqueued (not awaited) so the next recorder starts immediately,
            // but applied strictly in order — see enqueuePost above.
            if (sawSpeech && dur >= MIN_SEGMENT_MS) {
                enqueuePost(blob);
            }
            // Restart for the next utterance while still listening.
            if (isListening()) startRecorder();
        };
        recorder.start();
    };

    const pollLevel = () => {
        if (!analyser) return;
        const buf = new Uint8Array(analyser.fftSize);
        analyser.getByteTimeDomainData(buf);
        let sum = 0;
        for (let i = 0; i < buf.length; i++) {
            const v = (buf[i] - 128) / 128;
            sum += v * v;
        }
        const rms = Math.sqrt(sum / buf.length);

        const now = Date.now();
        if (rms >= SILENCE_RMS) {
            sawSpeech = true;
            silenceStart = 0;
        } else if (sawSpeech) {
            if (silenceStart === 0) silenceStart = now;
            else if (now - silenceStart >= SILENCE_MS) {
                cutSegment(); // utterance boundary
                return;
            }
        }
        // Hard cap so a long monologue still gets transcribed in chunks.
        if (now - segmentStart >= MAX_SEGMENT_MS && sawSpeech) {
            cutSegment();
        }
    };

    const stopCapture = async () => {
        isListening._set(false);
        currentTargetId._set(null);
        if (levelTimer != null) {
            clearInterval(levelTimer);
            levelTimer = null;
        }
        if (recorder && recorder.state !== "inactive") {
            try {
                recorder.onstop = null as any; // don't restart on a deliberate stop
                recorder.stop();
            } catch {
                /* ignore */
            }
        }
        recorder = null;
        if (audioCtx) {
            try { await audioCtx.close(); } catch { /* ignore */ }
            audioCtx = null;
        }
        analyser = null;
        if (stream) {
            stream.getTracks().forEach((t) => t.stop());
            stream = null;
        }
        activeHandle?.setInterim("");
    };

    const startCapture = async () => {
        try {
            stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        } catch (e: any) {
            const name = e?.name || "";
            if (name === "NotAllowedError" || name === "SecurityError") {
                fail("not-allowed");
            } else if (name === "NotFoundError" || name === "OverconstrainedError") {
                fail("audio-capture");
            } else {
                fail("service-not-allowed");
            }
            return;
        }
        mime = pickMime();
        lastError._set(null);
        isListening._set(true);

        audioCtx = new AudioContext();
        const src = audioCtx.createMediaStreamSource(stream);
        analyser = audioCtx.createAnalyser();
        analyser.fftSize = 2048;
        src.connect(analyser);

        startRecorder();
        levelTimer = window.setInterval(pollLevel, LEVEL_POLL_MS);
    };

    const toggleListening = () => {
        if (isListening()) {
            void stopCapture();
        } else {
            void startCapture();
        }
    };

    return {
        isListening,
        currentTargetId,
        lastError,
        isAvailable: () => true,
        toggleListening,
        registerPane: (blockId, handle) => {
            activeHandle = handle;
            currentTargetId._set(blockId);
        },
    };
}
