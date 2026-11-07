// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The in-app login panel's state: what the user pasted, whether it is being sent, what came back, and the actions
 * that send a code (typed, pasted into the box, or read from the clipboard).
 */

import { createEffect, createSignal, type Accessor } from "solid-js";
import { loginBackend } from "../flows/login-backend";
import { readText as clipboardReadText } from "@/util/clipboard";

/** Live phase of the in-app login session, surfaced as a status line. */
export type InAppLoginPhase = "starting" | "waiting-authorize" | "fallback" | "terminal-polling";

export interface InAppLoginPanelProps {
    providerId: string;
    providerLabel: string;
    authUrl: string | undefined;
    phase: InAppLoginPhase;
    onCancel: () => void;
    onUseTerminal: () => void;
}

export type LoginPasteState = {
    pasteCode: Accessor<string>;
    setPasteCode: (code: string) => void;
    pasting: Accessor<boolean>;
    pasteResult: Accessor<string | null>;
    setInputRef: (el: HTMLInputElement) => void;
    submitCode: (explicit?: string) => Promise<void>;
    pasteFromClipboard: () => void;
};

export function phaseText(phase: InAppLoginPhase, providerLabel: string): string {
    switch (phase) {
        case "starting":
            return `Starting ${providerLabel} sign-in — requesting a sign-in link…`;
        case "waiting-authorize":
            return "Waiting for you to authorize in the browser — we'll detect it automatically.";
        case "fallback":
            return "No sign-in link appeared — trying your existing login, then a terminal…";
        case "terminal-polling":
            return "A terminal window opened — finish the login there.";
    }
}

export function createLoginPaste(p: InAppLoginPanelProps): LoginPasteState {
    const [pasteCode, setPasteCode] = createSignal("");
    const [pasting, setPasting] = createSignal(false);
    const [pasteResult, setPasteResult] = createSignal<string | null>(null);
    let inputRef: HTMLInputElement | undefined;

    // Grab focus when the paste input appears (same rationale as
    // AuthUrlBox/AgentDocumentView: a pasted code must land HERE, not
    // whatever field held focus before). Deferred a frame so it wins the
    // host's own focus management; fires once per URL appearance.
    createEffect(() => {
        if (p.authUrl) {
            requestAnimationFrame(() => inputRef?.focus());
        }
    });

    const submitCode = async (explicit?: string): Promise<void> => {
        // Read from the live input element as a fallback (mirrors
        // AuthUrlBox): if focus desynced and the controlled signal missed
        // the paste, the DOM value still holds it.
        const code = (explicit ?? inputRef?.value ?? pasteCode()).trim();
        if (!code) return;
        setPasting(true);
        setPasteResult(null);
        try {
            // Delivered to the login child's stdin via the host's
            // set_provider_auth → CliLoginStdin::write_line plumbing; the
            // session's completion poll (inside runProviderLogin) then
            // observes the CLI finishing. SECURITY: the code is single-use
            // and PKCE-bound to the spawned process (spec §5) — never logged.
            await loginBackend().submitCode(p.providerId, code);
            setPasteResult("Code accepted — signing you in…");
            setPasteCode("");
        } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            setPasteResult(`Error: ${msg}`);
        } finally {
            setPasting(false);
        }
    };

    const pasteFromClipboard = (): void => {
        clipboardReadText()
            .then((text) => {
                const trimmed = (text ?? "").trim();
                if (trimmed) {
                    setPasteCode(trimmed);
                    void submitCode(trimmed);
                }
            })
            .catch(() => {
                setPasteResult("Could not read clipboard — paste manually");
            });
    };

    return {
        pasteCode,
        setPasteCode,
        pasting,
        pasteResult,
        setInputRef: (el) => {
            inputRef = el;
        },
        submitCode,
        pasteFromClipboard,
    };
}
