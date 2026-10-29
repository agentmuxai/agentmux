// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Sign in to AgentMux Cloud to connect GitHub", with the Cloud sign-in itself (the same
// choices and flow as Settings → MuxBus). Shown in place of Connect while there is no
// Cloud sign-in; once signed in, the status turns connected and the Connect button is back.

import { onMount, Show, type JSX } from "solid-js";

import { useMuxBusStatus } from "./AgentMuxConnectPanel";
import { GITHUB_CLOUD_SIGN_IN_TEXT } from "./github-cloud-gate";
import { MuxBusSignInChoices } from "./muxbus-sign-in-choices";

export function GithubCloudSignIn(): JSX.Element {
    const muxbus = useMuxBusStatus();
    onMount(() => void muxbus.refresh());
    return (
        <div class="oauth-cloud-signin" role="status">
            <div class="oauth-cloud-signin-text">{GITHUB_CLOUD_SIGN_IN_TEXT}</div>
            <div class="oauth-cloud-signin-note">
                GitHub tokens for your agents come from AgentMux Cloud, so this needs your AgentMux Cloud account.
            </div>
            <MuxBusSignInChoices
                identityProviders={muxbus.identityProviders()}
                loading={muxbus.loading()}
                disabled={!muxbus.isConfigured()}
                label="Sign in"
                cancelLabel="Cancel sign-in"
                onSignIn={(provider) => void muxbus.connect(provider)}
                onCancel={() => void muxbus.cancel()}
            />
            <Show when={muxbus.error()}>
                {(error) => (
                    <div class="identity-form-error" role="alert">
                        {error()}
                    </div>
                )}
            </Show>
        </div>
    );
}
