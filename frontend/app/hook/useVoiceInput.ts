// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignalAtom, type SignalAtom } from "@/util/util";

export interface PaneVoiceHandle {
    appendFinal: (text: string) => void;
    setInterim: (text: string) => void;
}

export interface VoiceSession {
    isListening: SignalAtom<boolean>;
    /** blockId of the pane currently receiving voice output (null when no
     *  pane is targeted). Each pane's mic button reads this to determine
     *  whether *it* owns the active session — multi-pane safe. */
    currentTargetId: SignalAtom<string | null>;
    /** Last fatal recognition error code (e.g. "not-allowed",
     *  "audio-capture", "service-not-allowed"), or null when none / cleared.
     *  Mic buttons read this to render a "blocked" affordance; cleared when a
     *  session starts successfully. */
    lastError: SignalAtom<string | null>;
    isAvailable: () => boolean;
    toggleListening: () => void;
    registerPane: (blockId: string, handle: PaneVoiceHandle) => void;
}

const RESTART_DELAY_MS = 100;

function createVoiceSession(): VoiceSession {
    const SR = (window as any).SpeechRecognition ?? (window as any).webkitSpeechRecognition;
    const isListening = createSignalAtom(false);
    const currentTargetId = createSignalAtom<string | null>(null);
    const lastError = createSignalAtom<string | null>(null);

    if (!SR) {
        return {
            isListening,
            currentTargetId,
            lastError,
            isAvailable: () => false,
            toggleListening: () => {},
            registerPane: () => {},
        };
    }

    const recognition: SpeechRecognition = new SR();
    recognition.continuous = true;
    recognition.interimResults = true;
    recognition.lang = navigator.language;

    let activeHandle: PaneVoiceHandle | null = null;
    let interimActive = false;

    recognition.onresult = (event: SpeechRecognitionEvent) => {
        let interim = "";
        let finals = "";

        for (let i = event.resultIndex; i < event.results.length; i++) {
            const result = event.results[i];
            if (result.isFinal) {
                finals += result[0].transcript;
            } else {
                interim += result[0].transcript;
            }
        }

        if (finals) {
            activeHandle?.setInterim("");
            interimActive = false;
            activeHandle?.appendFinal(finals);
        }

        if (interim) {
            activeHandle?.setInterim(interim);
            interimActive = true;
        } else if (!finals) {
            activeHandle?.setInterim("");
            interimActive = false;
        }
    };

    recognition.onerror = (event: SpeechRecognitionErrorEvent) => {
        // Fatal errors that should stop the session and surface guidance:
        //   not-allowed         — mic permission denied (OS or CEF layer)
        //   service-not-allowed — recognition service unavailable (the Web
        //                         Speech backend doesn't work in CEF; #1591)
        //   audio-capture       — no microphone device present
        const FATAL = ["not-allowed", "service-not-allowed", "audio-capture"];
        if (FATAL.includes(event.error)) {
            isListening._set(false);
            currentTargetId._set(null);
            lastError._set(event.error);
            window.dispatchEvent(new CustomEvent("voice-input-error", { detail: event.error }));
        }
        // "no-speech" and "aborted" are non-fatal; onend auto-restarts.
    };

    recognition.onend = () => {
        if (isListening()) {
            setTimeout(() => {
                if (isListening()) {
                    try { recognition.start(); } catch { /* already started */ }
                }
            }, RESTART_DELAY_MS);
        }
    };

    const toggleListening = () => {
        if (isListening()) {
            recognition.stop();
            if (interimActive) {
                activeHandle?.setInterim("");
                interimActive = false;
            }
            isListening._set(false);
            // Clear target so per-pane mic buttons reflect "no active
            // session" instead of leaving the previous target's button
            // stuck in active state.
            currentTargetId._set(null);
        } else {
            try {
                recognition.start();
                isListening._set(true);
                // A fresh start clears any prior blocked/error state so the
                // mic button drops its "blocked" affordance on retry.
                lastError._set(null);
            } catch {
                // Race with onend auto-restart — ignore.
            }
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

let _session: VoiceSession | null = null;

export function getVoiceSession(): VoiceSession {
    if (!_session) _session = createVoiceSession();
    return _session;
}
