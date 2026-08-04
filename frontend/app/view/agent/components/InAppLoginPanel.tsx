// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InAppLoginPanel — the in-app OAuth login UI (spec §3.1's session, rendered):
 * URL + Open/Copy, a paste-code box wired to `setProviderAuth`, a live phase
 * line, and Cancel / "Use terminal instead" actions.
 *
 * Extracted from `PreLaunchAuthPanel.tsx` (its original and still-primary
 * caller — the launch surface, spec §3.3 surface 1) so the Armory/Stash
 * surface (§3.3 surface 3) can reuse the identical UI instead of a second
 * hand-rolled copy. Decoupled from `PreLaunchAuthPanel`'s own
 * `AuthFlowController`/`AuthState` reducer — callers pass `authUrl` directly
 * (the only field of that reducer's state this component ever read).
 *
 * See docs/specs/SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md.
 */

import { Button } from "@/element/button";
import { getApi } from "@/app/store/global";
import { readText as clipboardReadText, writeText as clipboardWriteText } from "@/util/clipboard";
import { createEffect, createSignal, Show, type JSX } from "solid-js";

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

export const InAppLoginPanel = (p: InAppLoginPanelProps): JSX.Element => {
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
            await getApi().setProviderAuth(p.providerId, code);
            setPasteResult("Code accepted — signing you in…");
            setPasteCode("");
        } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            setPasteResult(`Error: ${msg}`);
        } finally {
            setPasting(false);
        }
    };

    const phaseText = (): string => {
        switch (p.phase) {
            case "starting":
                return `Starting ${p.providerLabel} sign-in — requesting a sign-in link…`;
            case "waiting-authorize":
                return "Waiting for you to authorize in the browser — we'll detect it automatically.";
            case "fallback":
                return "No sign-in link appeared — trying your existing login, then a terminal…";
            case "terminal-polling":
                return "A terminal window opened — finish the login there.";
        }
    };

    return (
        <div class="pre-launch-auth-panel-waiting">
            <div class="pre-launch-auth-panel-waiting-title">
                🔐 Sign in to {p.providerLabel}
            </div>
            <div class="pre-launch-auth-panel-hint">{phaseText()}</div>
            {/* reagent P1 on PR #2410: gate on phase too, not just authUrl's
                presence. "Use terminal instead" kills the tier-1 login child
                (cancelCliLogin) and moves phase to "fallback"/"terminal-
                polling" WITHOUT clearing the now-dead authUrl — without this
                gate, the URL/paste box stayed visible for a session that no
                longer exists. Pasting a code at that point would call
                setProviderAuth, whose host handler falls through to the
                non-CLI config-file branch (cli_login_stdin already cleared)
                and silently writes the code as a plain auth_token while
                showing "Code accepted…", misleading the user while the real
                login runs in the separately-opened terminal instead. */}
            <Show
                when={
                    p.phase === "starting" || p.phase === "waiting-authorize"
                        ? p.authUrl
                        : undefined
                }
            >
                {(url) => (
                    <>
                        <div class="pre-launch-auth-panel-url-label">
                            1 · Authorize in your browser (it should have opened — if not, use this link):
                        </div>
                        <div class="pre-launch-auth-panel-url-row">
                            <code class="pre-launch-auth-panel-url-text" title={url()}>
                                {url()}
                            </code>
                            <Button
                                className="grey solid"
                                onClick={() => {
                                    try {
                                        getApi().openExternal(url());
                                    } catch (e) {
                                        console.warn(`[auth-diag] openExternal failed: ${(e as Error)?.message ?? String(e)}`);
                                    }
                                }}
                            >
                                Open
                            </Button>
                            <Button
                                className="grey solid"
                                onClick={() => {
                                    // CEF clipboard wrapper, not navigator.clipboard —
                                    // SPEC_UNIFIED_CLIPBOARD_2026_05_18.md §3.3.
                                    void clipboardWriteText(url()).catch((err) =>
                                        console.log("clipboard write failed", err),
                                    );
                                }}
                            >
                                Copy
                            </Button>
                        </div>
                        <div class="pre-launch-auth-panel-url-label">
                            2 · If the page shows an authorization code, paste it here:
                        </div>
                        <div class="pre-launch-auth-panel-callback-row">
                            <input
                                ref={inputRef}
                                class="pre-launch-auth-panel-url-input"
                                type="text"
                                placeholder="Paste the authorization code…"
                                value={pasteCode()}
                                onInput={(e) => setPasteCode(e.currentTarget.value)}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter") void submitCode();
                                }}
                                onPaste={(e) => {
                                    // Auto-submit on paste — pasting the code IS the
                                    // intent to submit (same UX as AuthUrlBox).
                                    const text = (e.clipboardData?.getData("text") ?? "").trim();
                                    if (text) {
                                        setPasteCode(text);
                                        void submitCode(text);
                                    }
                                }}
                            />
                            <Button
                                className="grey solid"
                                title="Paste from clipboard and submit"
                                onClick={() => {
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
                                }}
                            >
                                Paste &amp; submit
                            </Button>
                            <Button
                                className="grey solid"
                                onClick={() => void submitCode()}
                                disabled={pasting()}
                            >
                                {pasting() ? "…" : "Submit"}
                            </Button>
                        </div>
                        <Show when={pasteResult()}>
                            <div class="pre-launch-auth-panel-hint">{pasteResult()}</div>
                        </Show>
                    </>
                )}
            </Show>
            <div class="pre-launch-auth-panel-inapp-actions">
                <Button onClick={() => p.onCancel()}>Cancel login</Button>
                {/* Explicit terminal fallback — never auto-launched (spec
                    §3.2). Hidden once the flow is already in the terminal
                    tiers, where the request would only abort a live poll. */}
                <Show when={p.phase === "starting" || p.phase === "waiting-authorize"}>
                    <Button className="grey" onClick={() => p.onUseTerminal()}>
                        Use terminal instead
                    </Button>
                </Show>
            </div>
        </div>
    );
};
