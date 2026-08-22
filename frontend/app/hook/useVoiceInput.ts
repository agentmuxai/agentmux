// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignalAtom, type SignalAtom } from "@/util/util";
import { getSettingsKeyAtom } from "@/app/store/global";
import { createWhisperVoiceSession } from "./whisperVoiceEngine";

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
    /** Optional detail string accompanying `lastError` — e.g. the server's
     *  actual error body ("whisper-cli not found at ...") for
     *  `service-not-allowed`, rather than just the coarse category. Additive:
     *  existing consumers (MicButton.tsx's tooltip) only read `lastError` and
     *  are unaffected; the Settings -> Recording "test your microphone" flow
     *  reads this to show the specific failure. Not implemented by every
     *  engine (optional) — the Web Speech engine has no server round-trip to
     *  report a detail for. */
    lastErrorDetail?: SignalAtom<string | null>;
    isAvailable: () => boolean;
    toggleListening: () => void;
    registerPane: (blockId: string, handle: PaneVoiceHandle) => void;
}

const RESTART_DELAY_MS = 100;

function createWebSpeechVoiceSession(): VoiceSession {
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

type VoiceEngineMode = "webspeech" | "whisper-local" | "groq";

let _session: VoiceSession | null = null;
// Which resolved mode `_session` was actually built for. Must be the FULL
// three-way resolution, not just "webspeech vs. whisper" — createWhisperVoiceSession()
// itself branches on "groq" vs. "whisper-local" internally (a captured-once
// `isLocal` controlling webm vs. 16kHz-WAV capture), so a groq<->whisper-local
// change needs a rebuild too, even though both route through that same
// factory function. Collapsing them to one "whisper" bucket here would miss
// exactly that transition.
let _sessionMode: VoiceEngineMode | null = null;

function resolveVoiceEngineMode(): VoiceEngineMode {
    // Engine selection (SPEC_VOICE_STT_ENGINE_2026_06_20.md §5): the Web Speech
    // recognizer can't transcribe in CEF (closed-source Google service), so it's
    // used ONLY when explicitly opted into via `voice:engine: "webspeech"` AND
    // the API exists (dev / real-Chromium). Otherwise the Whisper
    // capture-and-send engine, which is the default everywhere.
    const engine = getSettingsKeyAtom("voice:engine")();
    const hasWebSpeech =
        typeof window !== "undefined" &&
        !!((window as any).SpeechRecognition ?? (window as any).webkitSpeechRecognition);
    if (engine === "webspeech" && hasWebSpeech) return "webspeech";
    if (engine === "whisper-local") return "whisper-local";
    return "groq";
}

/**
 * Settings -> Recording's engine picker (#2751) made `voice:engine` a live,
 * user-facing switch for the first time — previously it was settings.json-only
 * and effectively required an app restart to notice. Rebuilds the underlying
 * session (stopping any in-progress capture cleanly) when the resolved engine
 * no longer matches what it was built for.
 */
function ensureVoiceSession(): VoiceSession {
    const resolvedMode = resolveVoiceEngineMode();
    if (_session && _sessionMode !== resolvedMode) {
        if (_session.isListening()) _session.toggleListening();
        _session = null;
    }
    if (!_session) {
        _session = resolvedMode === "webspeech" ? createWebSpeechVoiceSession() : createWhisperVoiceSession();
        _sessionMode = resolvedMode;
    }
    return _session;
}

const NULL_STRING_SIGNAL: SignalAtom<string | null> = createSignalAtom<string | null>(null);

/** A SignalAtom that always reads/writes through to whichever session is
 * CURRENT at call time (via `ensureVoiceSession()`), rather than one fixed
 * session captured at creation. This is what makes the facade below safe for
 * long-lived callers to cache once: `resolveVoiceEngineMode()` reads the live
 * `voice:engine` setting on every read, so any reactive scope that calls a
 * facade signal (e.g. a component's JSX) automatically re-subscribes to
 * whichever underlying session is current, and re-runs when the engine
 * setting itself changes — not just when the old session's own signal fires
 * (which stops happening the moment that session is discarded). */
function facadeSignal<T>(pick: (s: VoiceSession) => SignalAtom<T> | undefined, fallback: SignalAtom<T>): SignalAtom<T> {
    const read = (() => (pick(ensureVoiceSession()) ?? fallback)()) as SignalAtom<T>;
    (read as any)._set = (v: T) => (pick(ensureVoiceSession()) ?? fallback)._set(v);
    return read;
}

let _facade: VoiceSession | null = null;

/**
 * Returns a STABLE facade object (same identity forever) so long-lived
 * callers that cache the return value once at component setup — MicButton.tsx,
 * AgentFooter.tsx, both `const voice = getVoiceSession()` at the top of the
 * component body, not re-invoked per render — still always operate on
 * whichever underlying session is current. An earlier version of this fix
 * (this same PR) rebuilt the underlying session correctly but returned it
 * directly, so only call sites that invoke `getVoiceSession()` fresh on every
 * use (keymodel.ts's global hotkey handler) actually observed the rebuild;
 * the primary per-pane mic-button path did not. Found in PR #2751 re-review.
 */
export function getVoiceSession(): VoiceSession {
    if (_facade) return _facade;
    _facade = {
        isListening: facadeSignal((s) => s.isListening, createSignalAtom(false)),
        currentTargetId: facadeSignal((s) => s.currentTargetId, createSignalAtom<string | null>(null)),
        lastError: facadeSignal((s) => s.lastError, NULL_STRING_SIGNAL),
        lastErrorDetail: facadeSignal((s) => s.lastErrorDetail, NULL_STRING_SIGNAL),
        isAvailable: () => ensureVoiceSession().isAvailable(),
        toggleListening: () => ensureVoiceSession().toggleListening(),
        registerPane: (blockId, handle) => ensureVoiceSession().registerPane(blockId, handle),
    };
    return _facade;
}
