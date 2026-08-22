// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Live microphone level meter for the Settings -> Recording "Test your
 * microphone" flow. Reuses the same RMS-from-AnalyserNode computation
 * `whisperVoiceEngine.ts`'s VAD already performs (`audioLevel.ts`), applied
 * here purely for display rather than silence-detection.
 *
 * Independent of `whisperVoiceEngine.ts`'s own capture graph — this owns its
 * own short-lived AudioContext/AnalyserNode against whatever stream the
 * caller hands it (the test flow's own getUserMedia call), so it doesn't
 * interfere with an actual in-progress voice-input session elsewhere.
 *
 * See docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md §4.
 */
import { onCleanup } from "solid-js";
import { createSignalAtom, type SignalAtom } from "@/util/util";
import { computeRms } from "./audioLevel";

const POLL_MS = 100;

export interface MicLevelMeter {
    /** Current RMS level, 0..~1. Resets to 0 when stopped. */
    level: SignalAtom<number>;
    /** Start polling the given stream's audio level. Safe to call repeatedly (restarts). */
    start: (stream: MediaStream) => void;
    /** Stop polling and release the AudioContext. Safe to call when not started. */
    stop: () => void;
}

export function createMicLevelMeter(): MicLevelMeter {
    const level = createSignalAtom(0);
    let audioCtx: AudioContext | null = null;
    let analyser: AnalyserNode | null = null;
    let timer: number | null = null;

    const stop = () => {
        if (timer != null) {
            clearInterval(timer);
            timer = null;
        }
        analyser = null;
        if (audioCtx) {
            const ctx = audioCtx;
            audioCtx = null;
            void ctx.close().catch(() => {});
        }
        level._set(0);
    };

    const start = (stream: MediaStream) => {
        stop();
        try {
            audioCtx = new AudioContext();
            const src = audioCtx.createMediaStreamSource(stream);
            analyser = audioCtx.createAnalyser();
            analyser.fftSize = 2048;
            src.connect(analyser);
            timer = window.setInterval(() => {
                if (analyser) level._set(computeRms(analyser));
            }, POLL_MS);
        } catch {
            stop();
        }
    };

    onCleanup(stop);

    return { level, start, stop };
}
