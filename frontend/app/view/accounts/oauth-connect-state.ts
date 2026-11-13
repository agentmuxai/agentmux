// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The state of an OAuth connect (OAuthConnectPanel.tsx): the client, the flow's session and status,
// the polling, Connect, Reconnect's auto-start, Replace and Cancel. The pieces are made in the order the
// panel always made them: the order effects and the clean-up are registered in is part of what it does.

import { createEffect, createSignal, on, onCleanup } from "solid-js";

import { RpcApi, type OAuthFlowStatus } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openLink } from "@/app/store/global";
import type { AccountProvider } from "@/app/view/identity/identity-model";
import { PROVIDER_LABELS, refreshAccountCache } from "@/app/view/identity/identity-model";
import type { AccountOAuthClientInfo } from "@/types/rpc/AccountOAuthClientInfo";
import { muxbusDelivery, muxbusOffersSignIn } from "@/store/muxbus-delivery";
import { githubNeedsCloudSignIn, isNeedsCloudSignInError } from "./github-cloud-gate";
import { isBlockedError, setOAuthNote } from "./oauth-account-state";
import type { OAuthServiceInfo } from "./oauth-catalog";
import { clientUnavailable, createOAuthClientState } from "./OAuthClientSection";

const POLL_INTERVAL_MS = 2000;

/** What a finished connect reports. */
export interface OAuthConnectResult {
    accountId: string;
    /** Scopes asked for and unticked on the consent screen. */
    missingScopes: string[];
    email?: string | null;
}

export interface OAuthConnectPanelProps {
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
    /** Connect with AgentMux's own client, whatever client the person saved (the GitHub App choice). */
    useBuiltinClient?: boolean;
    /** Called once the backend reports success (account already created or updated). */
    onConnected: (result: OAuthConnectResult) => void;
    /** Shows a Cancel button beside Connect. */
    onCancel?: () => void;
}

/** The flow's session, its status, and the polling that follows it to its end. */
function createFlowSession(props: OAuthConnectPanelProps) {
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

    return {
        sessionId, setSessionId, status, setStatus, busy, setBusy, localError, setLocalError,
        stopPolling, applyStatus,
        /** A new connect: the URL may open again, and polling may run. */
        begin: () => {
            opened = false;
            stopped = false;
        },
        /** Back to the start: the URL may open again. */
        forgetOpened: () => {
            opened = false;
        },
    };
}

/** Nothing to connect with: AgentMux's own client is the one asked for and this build has none, or the provider hides its client and there is none. */
function connectUnavailable(info: OAuthServiceInfo, client: AccountOAuthClientInfo | null, builtinOnly: boolean): boolean {
    return builtinOnly ? client != null && !client.builtinAvailable : clientUnavailable(info, client);
}

/** The OAuth connect's state: everything the panel's markup reads, and what it does. */
export function createOAuthConnect(props: OAuthConnectPanelProps, info: OAuthServiceInfo) {
    const client = createOAuthClientState(info, props.clientIdHint, props.useBuiltinClient === true);
    const flow = createFlowSession(props);
    const { sessionId, setSessionId, status, setStatus, setBusy, localError, setLocalError, stopPolling, applyStatus } = flow;

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
        flow.begin();
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

    // GitHub through AgentMux's own App needs the Cloud sign-in: say so, with the sign-in, in place of Connect.
    const needsCloud = () =>
        githubNeedsCloudSignIn(
            props.provider,
            client.state.loaded() &&
                !client.state.showFields() &&
                (props.useBuiltinClient === true || client.state.info()?.saved !== true),
            muxbusDelivery(),
        );

    const unavailable = () => connectUnavailable(info, client.state.info(), props.useBuiltinClient === true);

    // A Reconnect starts by itself once it knows it has a client to use, and, for GitHub, a Cloud sign-in.
    let autoStarted = false;
    createEffect(() => {
        if (!props.autoStart || autoStarted || !client.state.loaded() || client.state.showFields() || needsCloud() || unavailable()) return;
        autoStarted = true;
        void connect();
    });

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
        flow.forgetOpened();
    };

    // The person signed in from the prompt: the answer that sent them there no longer holds.
    createEffect(
        on(muxbusDelivery, (delivery) => {
            if (delivery == null || muxbusOffersSignIn(delivery)) return;
            if (isNeedsCloudSignInError(localError())) setLocalError(null);
            const s = status();
            if (s?.status === "failed" && isNeedsCloudSignInError(s.error)) setStatus(null);
        }),
    );

    const label = PROVIDER_LABELS[props.provider] ?? props.provider;

    return { client, status, localError, busy: flow.busy, needsCloud, unavailable, connect, replaceUser, reset, label };
}

/** What the panel's markup reads. */
export type OAuthConnectState = ReturnType<typeof createOAuthConnect>;
