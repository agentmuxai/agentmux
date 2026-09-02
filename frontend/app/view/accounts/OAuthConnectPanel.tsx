// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * OAuthConnectPanel — drives an Armory service-OAuth connect flow from the
 * Accounts form. The backend (oauth_client.rs + account.oauth.* RPCs) does all
 * the work: it opens the browser / emits a device code, exchanges tokens, and
 * creates the account (keychain-backed). This panel just kicks it off and polls
 * to completion, rendering the device code / browser prompt along the way.
 *
 * Built-in public client ids aren't provisioned yet, so the GitHub reference
 * provider uses the BYO path (user supplies their own OAuth app's client id).
 * docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §4.2/§12.1.
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { RpcApi, type OAuthFlowStatus } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openLink } from "@/app/store/global";
import { writeText } from "@/util/clipboard";
import type { AccountProvider } from "@/app/view/identity/identity-model";
import { refreshAccountCache } from "@/app/view/identity/identity-model";
import { oauthInfo, needsByo } from "./oauth-catalog";

const POLL_INTERVAL_MS = 2000;

interface OAuthConnectPanelProps {
    provider: AccountProvider;
    /** Reactive account name from the parent form. */
    name: () => string;
    /** Called once the backend reports success (account already created). */
    onConnected: () => void;
}

export function OAuthConnectPanel(props: OAuthConnectPanelProps): JSX.Element {
    const info = oauthInfo(props.provider);
    // Defensive: the parent only renders this for OAuth-capable providers.
    if (!info) return <div class="identity-form-error">OAuth is not available for this provider.</div>;

    const [clientId, setClientId] = createSignal("");
    const [clientSecret, setClientSecret] = createSignal("");
    const [sessionId, setSessionId] = createSignal<string | null>(null);
    const [status, setStatus] = createSignal<OAuthFlowStatus | null>(null);
    const [busy, setBusy] = createSignal(false);
    const [localError, setLocalError] = createSignal<string | null>(null);

    let pollTimer: number | undefined;
    let opened = false; // guard so we auto-open the auth URL only once
    let stopped = false;

    const stopPolling = () => {
        stopped = true;
        if (pollTimer !== undefined) {
            clearTimeout(pollTimer);
            pollTimer = undefined;
        }
    };

    const isRunning = (): boolean => {
        const s = status();
        return s != null && s.status !== "success" && s.status !== "failed";
    };

    onCleanup(() => {
        stopPolling();
        const sid = sessionId();
        // Best-effort: release the backend session if we leave mid-flow.
        if (sid && isRunning()) void RpcApi.AccountOAuthCancelCommand(TabRpcClient, { sessionId: sid });
    });

    const finishSuccess = async () => {
        await refreshAccountCache();
        props.onConnected();
    };

    const applyStatus = (s: OAuthFlowStatus, sid: string) => {
        setStatus(s);
        if (s.status === "url-available" && !opened) {
            opened = true;
            openLink(s.authUrl);
        }
        if (s.status === "success") {
            stopPolling();
            void finishSuccess();
            return;
        }
        if (s.status === "failed") {
            stopPolling();
            return;
        }
        // pending / url-available / code-emitted — keep polling. `poll` is a
        // hoisted function declaration (below) so this forward reference is safe.
        pollTimer = window.setTimeout(() => void poll(sid), POLL_INTERVAL_MS);
    };

    async function poll(sid: string) {
        if (stopped) return;
        try {
            const s = await RpcApi.AccountOAuthPollCommand(TabRpcClient, { sessionId: sid });
            applyStatus(s, sid);
        } catch (e) {
            setStatus({ status: "failed", error: (e as Error)?.message ?? String(e) });
            stopPolling();
        }
    }

    const connect = async () => {
        const n = props.name().trim();
        if (!n) {
            setLocalError("Enter a name first");
            return;
        }
        if (needsByo(info) && !clientId().trim()) {
            setLocalError("Client ID is required");
            return;
        }
        if (info.requiresSecret && !clientSecret().trim()) {
            setLocalError("Client secret is required");
            return;
        }
        setBusy(true);
        setLocalError(null);
        opened = false;
        stopped = false;
        try {
            const res = await RpcApi.AccountOAuthStartCommand(TabRpcClient, {
                provider: props.provider,
                name: n,
                clientId: clientId().trim() || undefined,
                clientSecret: info.requiresSecret ? clientSecret().trim() || undefined : undefined,
            });
            if (res.error || !res.sessionId || !res.status) {
                setLocalError(res.error ?? "Could not start the OAuth flow");
                return;
            }
            setSessionId(res.sessionId);
            applyStatus(res.status, res.sessionId);
        } catch (e) {
            setLocalError((e as Error)?.message ?? String(e));
        } finally {
            setBusy(false);
        }
    };

    const reset = () => {
        stopPolling();
        const sid = sessionId();
        if (sid) void RpcApi.AccountOAuthCancelCommand(TabRpcClient, { sessionId: sid });
        setSessionId(null);
        setStatus(null);
        setLocalError(null);
        opened = false;
    };

    return (
        <div class="oauth-connect">
            <Show when={localError()}>
                <div class="identity-form-error">{localError()}</div>
            </Show>

            {/* Idle: BYO credentials + Connect */}
            <Show when={status() == null}>
                <Show when={needsByo(info)}>
                    <div class="oauth-byo-note">
                        ⓘ {info.byoHint ?? "Supply your own OAuth app's client id."}
                        <Show when={info.consoleUrl}>
                            {" "}
                            <a
                                class="oauth-link"
                                href="#"
                                onClick={(e) => {
                                    e.preventDefault();
                                    openLink(info.consoleUrl!);
                                }}
                            >
                                Open developer console ↗
                            </a>
                        </Show>
                    </div>
                    <FormRow label="Client ID">
                        <input
                            class="identity-input"
                            type="text"
                            value={clientId()}
                            onInput={(e) => setClientId(e.currentTarget.value)}
                            placeholder="Iv1.0123456789abcdef"
                        />
                    </FormRow>
                    <Show when={info.requiresSecret}>
                        <FormRow label="Client secret">
                            <input
                                class="identity-input"
                                type="password"
                                autocomplete="off"
                                value={clientSecret()}
                                onInput={(e) => setClientSecret(e.currentTarget.value)}
                                placeholder="stored in your OS keychain — never in plaintext"
                            />
                        </FormRow>
                    </Show>
                </Show>
                <div class="identity-key-actions">
                    <button
                        class="identity-btn identity-btn-primary"
                        disabled={busy()}
                        onClick={() => void connect()}
                    >
                        {busy() ? "Starting…" : `Connect with ${providerLabel(props.provider)}`}
                    </button>
                </div>
            </Show>

            {/* Running: device code or browser prompt */}
            <Show when={status()}>
                {(s) => (
                    <div class="oauth-flow-status">
                        <Show when={s().status === "code-emitted"}>
                            <div class="oauth-device">
                                <div class="oauth-device-label">Enter this code at the verification page:</div>
                                <div class="oauth-device-code-row">
                                    <code class="oauth-device-code">
                                        {(s() as { userCode: string }).userCode}
                                    </code>
                                    <button
                                        class="identity-btn identity-btn-secondary"
                                        onClick={() => void writeText((s() as { userCode: string }).userCode)}
                                    >
                                        Copy
                                    </button>
                                </div>
                                <button
                                    class="identity-btn identity-btn-primary"
                                    onClick={() => openLink((s() as { verificationUri: string }).verificationUri)}
                                >
                                    Open verification page ↗
                                </button>
                                <div class="oauth-waiting">Waiting for you to authorize…</div>
                            </div>
                        </Show>

                        <Show when={s().status === "pending" || s().status === "url-available"}>
                            <div class="oauth-waiting">
                                Waiting for browser authorization…
                                <Show when={s().status === "url-available"}>
                                    {" "}
                                    <a
                                        class="oauth-link"
                                        href="#"
                                        onClick={(e) => {
                                            e.preventDefault();
                                            openLink((s() as { authUrl: string }).authUrl);
                                        }}
                                    >
                                        Reopen browser ↗
                                    </a>
                                </Show>
                            </div>
                        </Show>

                        <Show when={s().status === "failed"}>
                            <div class="identity-form-error">
                                {(s() as { error: string }).error}
                            </div>
                        </Show>

                        <div class="identity-key-actions">
                            <Show
                                when={s().status === "failed"}
                                fallback={
                                    <button class="identity-btn identity-btn-secondary" onClick={reset}>
                                        Cancel
                                    </button>
                                }
                            >
                                <button class="identity-btn identity-btn-secondary" onClick={reset}>
                                    Try again
                                </button>
                            </Show>
                        </div>
                    </div>
                )}
            </Show>
        </div>
    );
}

OAuthConnectPanel.displayName = "OAuthConnectPanel";

function providerLabel(provider: AccountProvider): string {
    if (provider === "github") return "GitHub";
    if (provider === "aws") return "AWS";
    return provider.charAt(0).toUpperCase() + provider.slice(1);
}

function FormRow(props: { label: string; children: JSX.Element }): JSX.Element {
    return (
        <div class="identity-form-field">
            <label class="identity-form-label">{props.label}</label>
            {props.children}
        </div>
    );
}
