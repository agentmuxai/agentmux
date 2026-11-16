// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The sign-in part of an agent CLI's account details (Claude Code's, Codex's): when the provider no longer accepts the
// login it says so, and Reconnect runs the CLI's login again in the account's own directory
// (SPEC_CONNECTORS_EVERY_WAY_TO_CONNECT_2026_11_14.md §4.3). The account keeps its id, so the agents bound to it keep it.
// A service sign-in's details are OAuthAccountSection.

import { createEffect, createSignal, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import type { Account } from "@/app/view/identity/identity-model";
import { canReconnect, cliSignInOf, pendingReconnectRequest, takeReconnectRequest } from "./oauth-account-state";
import { ProviderLoginPanel } from "./ProviderLoginPanel";

export function CliSignInSection(props: { account: Account }): JSX.Element {
    return (
        <Show when={cliSignInOf(props.account)} keyed>
            {(signIn) => <CliSignInDetails account={props.account} brand={signIn.brand} provider={signIn.provider} />}
        </Show>
    );
}

function CliSignInDetails(props: { account: Account; brand: string; provider: string }): JSX.Element {
    const a = () => props.account;
    const [reconnecting, setReconnecting] = createSignal(false);
    // A Reconnect clicked on the card starts here, where the sign-in shows.
    createEffect(() => {
        if (canReconnect(a()) && pendingReconnectRequest() === a().id && takeReconnectRequest(a().id)) setReconnecting(true);
    });

    return (
        <Show when={canReconnect(a()) || reconnecting()}>
            <div class="identity-detail-section">{props.brand} sign-in</div>
            <Show when={!reconnecting()}>
                <div class="identity-form-warning">
                    Reconnect needed: {props.brand} no longer accepts this sign-in. Reconnect to keep using it; the agents on it keep it.
                </div>
            </Show>
            <Show
                when={reconnecting()}
                fallback={
                    <div class="identity-key-actions">
                        <Button tone="accent" icon="rotate-right" onClick={() => setReconnecting(true)}>
                            Reconnect
                        </Button>
                    </div>
                }
            >
                <ProviderLoginPanel
                    inline
                    provider={PROVIDERS[props.provider]}
                    brand={props.brand}
                    existingAccountId={a().id}
                    onClose={() => setReconnecting(false)}
                />
            </Show>
        </Show>
    );
}

CliSignInSection.displayName = "CliSignInSection";
