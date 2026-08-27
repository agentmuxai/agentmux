// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BrowserAuthModalPanel — prompts for HTTP Basic / Digest credentials
 * when CEF fires `browser-pane-auth-required` for a browser pane.
 *
 * Phase α of SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md.
 * Sibling to `AgentLaunchModalPanel` / `AgentInstallModalPanel` —
 * mounts inside the canonical `<ModalLayer>` via the `browser-auth` request kind.
 */

import { createSignal, onCleanup, type JSX } from "solid-js";

import { Button } from "@/element/button";

interface BrowserAuthModalPanelProps {
    origin: string;
    realm: string;
    isProxy: boolean;
    onCancel: () => void;
    onSubmit: (username: string, password: string, save: boolean) => void;
}

export const BrowserAuthModalPanel = (props: BrowserAuthModalPanelProps): JSX.Element => {
    const [username, setUsername] = createSignal("");
    const [password, setPassword] = createSignal("");
    // Unchecked by default — saving a credential to the OS keychain is an
    // opt-in action, not the default outcome of a one-off manual sign-in.
    const [saveCredential, setSaveCredential] = createSignal(false);
    // True once either footer button fired its handler. onCleanup
    // uses this to detect "modal was unmounted via ESC / backdrop
    // click / pane close / replace" and fire onCancel exactly once
    // so the parked CEF AuthCallback gets resolved — the layer's
    // safeClose() unmounts without routing through onCancel.
    let resolved = false;

    const submit = () => {
        resolved = true;
        props.onSubmit(username(), password(), saveCredential());
    };
    const cancel = () => {
        resolved = true;
        props.onCancel();
    };

    onCleanup(() => {
        if (!resolved) props.onCancel();
    });

    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            e.preventDefault();
            submit();
        } else if (e.key === "Escape") {
            e.preventDefault();
            cancel();
        }
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">
                    {props.isProxy ? "Proxy authentication required" : "Authentication required"}
                </h2>
                <p class="modal-panel-description">
                    <strong>{props.origin || "This site"}</strong>
                    {props.realm ? <> says: <em>{props.realm}</em></> : null}
                </p>
            </header>
            <div class="modal-panel-body browser-auth-modal-body">
                <label class="browser-auth-modal-field">
                    <span>Username</span>
                    <input
                        type="text"
                        class="browser-auth-modal-input"
                        autocomplete="username"
                        autofocus
                        value={username()}
                        onInput={(e) => setUsername(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                    />
                </label>
                <label class="browser-auth-modal-field">
                    <span>Password</span>
                    <input
                        type="password"
                        class="browser-auth-modal-input"
                        autocomplete="current-password"
                        value={password()}
                        onInput={(e) => setPassword(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                    />
                </label>
                <label class="browser-auth-modal-save-field">
                    <input
                        type="checkbox"
                        checked={saveCredential()}
                        onChange={(e) => setSaveCredential(e.currentTarget.checked)}
                    />
                    <span>Save this credential</span>
                </label>
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={cancel} data-modal-dismiss>Cancel</Button>
                <Button onClick={submit} className="green solid">Sign in</Button>
            </footer>
        </>
    );
};

BrowserAuthModalPanel.displayName = "BrowserAuthModalPanel";
