// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The sign-in part of a service-OAuth account's details: the client it was
// connected with, what was granted, why it stopped, and Reconnect, which
// re-runs the consent for what the account had
// (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §7
// "Status on the card"). Per-service switches come with G1.

import { createEffect, createSignal, on, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { PROVIDER_LABELS, refreshAccountCache, type Account } from "@/app/view/identity/identity-model";
import {
    canReconnect,
    clientModeTag,
    githubConnectedAs,
    githubGrantIsGone,
    isBrokeredGithub,
    isServiceOAuthAccount,
    oauthNote,
    partlyGrantedLine,
    pendingReconnectRequest,
    reconnectScopes,
    scopeLabel,
    takeReconnectRequest,
} from "./oauth-account-state";
import { OAuthConnectPanel } from "./OAuthConnectPanel";

export function OAuthAccountSection(props: { account: Account }): JSX.Element {
    return (
        <Show when={isServiceOAuthAccount(props.account)}>
            <SignInDetails account={props.account} />
        </Show>
    );
}

function SignInDetails(props: { account: Account }): JSX.Element {
    const a = () => props.account;
    const provider = () => PROVIDER_LABELS[a().provider] ?? a().provider;
    const [reconnecting, setReconnecting] = createSignal(false);
    // A Reconnect clicked on the card starts here, where the prompt shows.
    createEffect(() => {
        if (pendingReconnectRequest() === a().id && takeReconnectRequest(a().id)) setReconnecting(true);
    });
    const note = () => oauthNote(a().id);
    // A GitHub grant that was revoked on GitHub shows here as needing a reconnect: ask the broker
    // once when the details open, and srv marks the account if it is gone.
    createEffect(
        on(
            () => a().id,
            () => {
                if (!isBrokeredGithub(a()) || a().status === "needs_reauth") return;
                void RpcApi.GithubConnectionCommand(TabRpcClient, {})
                    .then((c) => {
                        if (githubGrantIsGone(c)) return refreshAccountCache();
                    })
                    .catch(() => {});
            },
        ),
    );
    const granted = () => [...new Set((a().context.granted_scopes ?? []).map(scopeLabel))];

    return (
        <>
            <div class="identity-detail-section">{provider()} sign-in</div>
            <Show when={githubConnectedAs(a())}>
                {(who) => <Field label="Connected as" value={who()} />}
            </Show>
            <Field label="Client" value={clientModeTag(a()) ?? "AgentMux"} />
            <Show when={a().context.oauth_client?.client_id}>
                {(id) => <Field label="Client ID" value={id()} />}
            </Show>
            <Show when={granted().length > 0}>
                <Field label="Allowed" value={granted().join(", ")} />
            </Show>
            <Show when={partlyGrantedLine(note()?.missingScopes ?? [])}>
                {(line) => <div class="identity-form-warning">{line()}</div>}
            </Show>
            <Show when={note()?.blocked}>
                {(error) => (
                    <div class="identity-form-error" role="alert">
                        <div class="oauth-error-title">Blocked by your organization</div>
                        {error()}
                    </div>
                )}
            </Show>
            <Show when={a().status === "needs_reauth" && !note()?.blocked}>
                <div class="identity-form-warning">
                    Reconnect needed: {provider()} no longer accepts this sign-in. Reconnect to keep using it.
                </div>
            </Show>
            <Show
                when={reconnecting()}
                fallback={
                    <Show when={canReconnect(a())}>
                        <div class="identity-key-actions">
                            <Button tone="accent" icon="rotate-right" onClick={() => setReconnecting(true)}>
                                Reconnect
                            </Button>
                        </div>
                    </Show>
                }
            >
                <OAuthConnectPanel
                    provider={a().provider}
                    accountId={a().id}
                    scopes={reconnectScopes(a())}
                    clientIdHint={a().context.oauth_client?.client_id}
                    autoStart
                    onConnected={() => setReconnecting(false)}
                    onCancel={() => setReconnecting(false)}
                />
            </Show>
        </>
    );
}

function Field(props: { label: string; value: string }): JSX.Element {
    return (
        <div class="identity-detail-field">
            <span class="identity-detail-label">{props.label}</span>
            <span class="identity-detail-value">{props.value}</span>
        </div>
    );
}
