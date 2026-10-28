// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * OAuthConnectPanel — drives a service-OAuth connect flow from the
 * Accounts form, and a Reconnect from an account's details. The backend
 * (oauth_client.rs + account.oauth.* RPCs) does all the work: it opens the
 * browser / emits a device code, exchanges tokens, and creates or updates the
 * account (keychain-backed). This panel picks the client
 * (`OAuthClientSection`), kicks the flow off and polls to completion,
 * rendering the device code / browser prompt along the way.
 *
 * Built-in public client ids aren't provisioned yet, so every provider uses
 * the user's own client, saved in the keychain after the first connect.
 * docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §4.2/§12.1,
 * docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §7.
 */

import { createEffect, createSignal, on, onCleanup, Show, type JSX } from "solid-js";
import { RpcApi, type OAuthFlowStatus } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openLink } from "@/app/store/global";
import { writeText } from "@/util/clipboard";
import type { AccountProvider } from "@/app/view/identity/identity-model";
import { PROVIDER_LABELS, refreshAccountCache } from "@/app/view/identity/identity-model";
import { oauthInfo } from "./oauth-catalog";
import { isBlockedError, setOAuthNote } from "./oauth-account-state";
import { createOAuthClientState, OAuthClientSection } from "./OAuthClientSection";
import { Button } from "@/app/element/ui";
import "./oauth-connect.scss";

const POLL_INTERVAL_MS = 2000;

/** What a finished connect reports. */
export interface OAuthConnectResult {
    accountId: string;
    /** Scopes asked for and unticked on the consent screen. */
    missingScopes: string[];
    email?: string | null;
}

interface OAuthConnectPanelProps {
    provider: AccountProvider;
    /** Reactive account name from the parent form; optional when the sign-in names the account. */
    name?: () => string;
    /** Authorize this existing account again (Reconnect) instead of creating one. */
    accountId?: string;
    /** Scopes beyond the provider's base ones. */
    scopes?: string[];
    /** Start once the client is known, without a Connect click (a Reconnect). */
    autoStart?: boolean;
    /** Prefills the client id if the user has to enter the client again. */
    clientIdHint?: string;
    /** Called once the backend reports success (account already created or updated). */
    onConnected: (result: OAuthConnectResult) => void;
    /** Shows a Cancel button beside Connect. */
    onCancel?: () => void;
}

export function OAuthConnectPanel(props: OAuthConnectPanelProps): JSX.Element {
    const info = oauthInfo(props.provider);
    // Defensive: the parent only renders this for OAuth-capable providers.
    if (!info) return <div class="identity-form-error">OAuth is not available for this provider.</div>;

    const client = createOAuthClientState(info, props.clientIdHint);
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
        return s != null && s.status !== "success" && s.status !== "failed" && s.status !== "confirm-user";
    };

    onCleanup(() => {
        stopPolling();
        const sid = sessionId();
        // Best-effort: release the backend session if we leave mid-flow.
        if (sid && isRunning()) void RpcApi.AccountOAuthCancelCommand(TabRpcClient, { sessionId: sid });
    });

    const finishSuccess = async (s: Extract<OAuthFlowStatus, { status: "success" }>) => {
        const missingScopes = s.missingScopes ?? [];
        setOAuthNote(s.accountId, { missingScopes });
        await refreshAccountCache();
        props.onConnected({ accountId: s.accountId, missingScopes, email: s.email });
    };

    const applyStatus = (s: OAuthFlowStatus, sid: string) => {
        setStatus(s);
        if (s.status === "url-available" && !opened) {
            opened = true;
            openLink(s.authUrl);
        }
        if (s.status === "success") {
            stopPolling();
            void finishSuccess(s);
            return;
        }
        if (s.status === "confirm-user") {
            stopPolling();
            return;
        }
        if (s.status === "failed") {
            stopPolling();
            // A reconnect its organization blocks shows on the account's card.
            if (props.accountId && isBlockedError(s.error)) setOAuthNote(props.accountId, { blocked: s.error });
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

    const connect = async (opts?: { replaceGithubUser?: boolean }) => {
        const n = props.name?.().trim() ?? "";
        if (!n && !props.accountId && !info.namedBySignIn) {
            setLocalError("Enter a name first");
            return;
        }
        const picked = client.state.requestFields();
        if ("error" in picked) {
            setLocalError(picked.error);
            return;
        }
        setBusy(true);
        setLocalError(null);
        opened = false;
        stopped = false;
        try {
            const scopes = props.scopes?.filter(Boolean) ?? [];
            const res = await RpcApi.AccountOAuthStartCommand(TabRpcClient, {
                provider: props.provider,
                name: props.accountId ? undefined : n || undefined,
                accountId: props.accountId,
                scopes: scopes.length > 0 ? scopes : undefined,
                replaceGithubUser: opts?.replaceGithubUser || undefined,
                ...picked.fields,
            });
            // srv holds the secret now; the input doesn't need it any more.
            client.state.clearSecret();
            if (res.error || !res.sessionId || !res.status) {
                const error = res.error ?? "Could not start the OAuth flow";
                if (props.accountId && isBlockedError(error)) setOAuthNote(props.accountId, { blocked: error });
                setLocalError(error);
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

    // A Reconnect starts by itself once it knows it has a client to use.
    let autoStarted = false;
    createEffect(
        on(client.state.loaded, (loaded) => {
            if (!props.autoStart || autoStarted || !loaded || client.state.showFields()) return;
            autoStarted = true;
            void connect();
        }),
    );

    // The person agreed to replace the GitHub user this account is connected as.
    const replaceUser = () => {
        stopPolling();
        setSessionId(null);
        setStatus(null);
        void connect({ replaceGithubUser: true });
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

    const label = PROVIDER_LABELS[props.provider] ?? props.provider;

    return (
        <div class="oauth-connect">
            <Show when={localError()}>
                {(err) => <FlowError error={err()} />}
            </Show>

            {/* Idle: the client + Connect */}
            <Show when={status() == null}>
                <OAuthClientSection client={client} />
                <div class="identity-key-actions">
                    <Button
                        tone="accent"
                        class="identity-btn identity-btn-primary"
                        disabled={busy() || !client.state.loaded()}
                        onClick={() => void connect()}
                    >
                        {busy() ? "Starting…" : props.accountId ? "Reconnect" : `Connect ${label}`}
                    </Button>
                    <Show when={props.onCancel}>
                        <Button class="identity-btn identity-btn-secondary" onClick={() => props.onCancel?.()}>
                            Cancel
                        </Button>
                    </Show>
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
                                    <Button
                                        class="identity-btn identity-btn-secondary"
                                        onClick={() => void writeText((s() as { userCode: string }).userCode)}
                                    >
                                        Copy
                                    </Button>
                                </div>
                                <Button
                                    tone="accent"
                                    class="identity-btn identity-btn-primary"
                                    onClick={() => openLink((s() as { verificationUri: string }).verificationUri)}
                                >
                                    Open verification page ↗
                                </Button>
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
                            <FlowError error={(s() as { error: string }).error} />
                        </Show>

                        <Show when={s().status === "confirm-user"}>
                            <ConfirmUser
                                status={s() as Extract<OAuthFlowStatus, { status: "confirm-user" }>}
                                onReplace={replaceUser}
                                onKeep={reset}
                            />
                        </Show>

                        <Show when={s().status !== "confirm-user"}>
                            <div class="identity-key-actions">
                                <Show
                                    when={s().status === "failed"}
                                    fallback={
                                        <Button class="identity-btn identity-btn-secondary" onClick={reset}>
                                            Cancel
                                        </Button>
                                    }
                                >
                                    <Button class="identity-btn identity-btn-secondary" onClick={reset}>
                                        Try again
                                    </Button>
                                </Show>
                            </div>
                        </Show>
                    </div>
                )}
            </Show>
        </div>
    );
}

OAuthConnectPanel.displayName = "OAuthConnectPanel";

/** GitHub: the person authorized as someone other than the user this AgentMux account is connected as. */
function ConfirmUser(props: {
    status: Extract<OAuthFlowStatus, { status: "confirm-user" }>;
    onReplace: () => void;
    onKeep: () => void;
}): JSX.Element {
    return (
        <div class="oauth-confirm-user" role="alert">
            <div class="oauth-error-title">Connect a different GitHub account?</div>
            <div>
                This AgentMux account is connected to GitHub as <b>@{props.status.currentLogin}</b>, and you just
                authorized <b>@{props.status.newLogin}</b>. Replacing it ends the repositories shared as @
                {props.status.currentLogin}; you can install on the new account's repositories afterwards.
            </div>
            <div class="identity-key-actions">
                <Button tone="accent" class="identity-btn identity-btn-primary" onClick={props.onReplace}>
                    Replace with @{props.status.newLogin}
                </Button>
                <Button class="identity-btn identity-btn-secondary" onClick={props.onKeep}>
                    Keep @{props.status.currentLogin}
                </Button>
            </div>
        </div>
    );
}

/** A flow's error; an organization's block gets its own heading (spec §7). */
function FlowError(props: { error: string }): JSX.Element {
    return (
        <div class="identity-form-error" role="alert">
            <Show when={isBlockedError(props.error)}>
                <div class="oauth-error-title">Blocked by your organization</div>
            </Show>
            {props.error}
        </div>
    );
}
