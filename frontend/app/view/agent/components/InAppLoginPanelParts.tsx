// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The in-app login panel's views: the link step, the paste step, and the Cancel / terminal actions. */

import { Button } from "@/element/button";
import { TextInput } from "@/app/element/ui/inputs";
import { getApi } from "@/app/store/global";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { Show, type Accessor, type JSX } from "solid-js";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";
import type { InAppLoginPanelProps, LoginPasteState } from "./InAppLoginPanelState";

/** Step 1: the sign-in link, to open in the browser or copy. */
export function AuthorizeStep(props: { url: Accessor<string> }): JSX.Element {
    const url = props.url;
    return (
        <div class="in-app-login-step">
            <div class="in-app-login-url-label">
                {t("login.authorize")}
            </div>
            <div class="in-app-login-hint">
                {t("login.authorizeHint")}
            </div>
            <div class="in-app-login-url-row">
                <code class="in-app-login-url-text" {...tip(url())}>
                    {url()}
                </code>
            </div>
            <div class="in-app-login-url-actions">
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
                    {t("login.openLink")}
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
                    {t("login.copyLink")}
                </Button>
            </div>
        </div>
    );
}

/** Step 2: the authorization code, typed, pasted into the box, or read from the clipboard. */
export function PasteStep(props: { s: LoginPasteState }): JSX.Element {
    const s = props.s;
    // A call, not an inline ternary: the compiler would wrap the ternary in a memo, which a button stand-in reading its children outside a render would warn about.
    const submitLabel = () => (s.pasting() ? "…" : t("login.submit"));
    return (
        <div class="in-app-login-step">
            <div class="in-app-login-url-label">
                {t("login.paste")}
            </div>
            <div class="in-app-login-hint">
                {t("login.pasteHint")}
            </div>
            <div class="in-app-login-callback-row">
                <TextInput
                    variant="plain"
                    ref={s.setInputRef}
                    class="in-app-login-url-input"
                    placeholder={t("login.pastePlaceholder")}
                    value={s.pasteCode()}
                    onInput={(e) => s.setPasteCode(e.currentTarget.value)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") void s.submitCode();
                    }}
                    onPaste={(e) => {
                        // Auto-submit on paste — pasting the code IS the
                        // intent to submit (same UX as AuthUrlBox).
                        const text = (e.clipboardData?.getData("text") ?? "").trim();
                        if (text) {
                            s.setPasteCode(text);
                            void s.submitCode(text);
                        }
                    }}
                />
                <Button
                    className="grey solid"
                    title={t("login.pasteSubmitTitle")}
                    onClick={() => s.pasteFromClipboard()}
                >
                    {t("login.pasteSubmit")}
                </Button>
                <Button
                    className="grey solid"
                    onClick={() => void s.submitCode()}
                    disabled={s.pasting()}
                >
                    {submitLabel()}
                </Button>
            </div>
            <Show when={s.pasteResult()}>
                <div class="in-app-login-hint">{s.pasteResult()}</div>
            </Show>
        </div>
    );
}

/** Cancel, and the explicit terminal fallback. */
export function LoginActions(props: { p: InAppLoginPanelProps }): JSX.Element {
    const p = props.p;
    return (
        <div class="in-app-login-actions">
            <Button onClick={() => p.onCancel()}>{t("login.cancel")}</Button>
            {/* Explicit terminal fallback — never auto-launched (spec
                §3.2). Hidden once the flow is already in the terminal
                tiers, where the request would only abort a live poll. */}
            <Show when={p.phase === "starting" || p.phase === "waiting-authorize"}>
                <Button className="grey" onClick={() => p.onUseTerminal()}>
                    {t("login.useTerminal")}
                </Button>
            </Show>
        </div>
    );
}
