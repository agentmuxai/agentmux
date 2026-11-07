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
 * Google and GitHub run on AgentMux's own client where the build has one (Google shows no client
 * fields at all); the other providers use the user's own client, saved in the keychain after the
 * first connect.
 * docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §4.2/§12.1,
 * docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §7.
 */

import { Show, type JSX } from "solid-js";
import { oauthInfo } from "./oauth-catalog";
import { isNeedsCloudSignInError } from "./github-cloud-gate";
import { createOAuthConnect, type OAuthConnectPanelProps } from "./oauth-connect-state";
import { FlowError, FlowStatusView, IdleRow } from "./oauth-connect-parts";
import "./oauth-connect.scss";

export type { OAuthConnectResult } from "./oauth-connect-state";

export function OAuthConnectPanel(props: OAuthConnectPanelProps): JSX.Element {
    const info = oauthInfo(props.provider);
    // Defensive: the parent only renders this for OAuth-capable providers.
    if (!info) return <div class="identity-form-error">OAuth is not available for this provider.</div>;

    const c = createOAuthConnect(props, info);

    return (
        <div class="oauth-connect">
            {/* srv's sign-in answer is the prompt below when the gate shows it already: not twice. */}
            <Show when={c.localError() && !(c.needsCloud() && isNeedsCloudSignInError(c.localError()))}>
                <FlowError error={c.localError()!} />
            </Show>

            {/* Idle: the client + Connect */}
            <Show when={c.status() == null}>
                <IdleRow c={c} panel={props} />
            </Show>

            {/* Running: device code or browser prompt */}
            <Show when={c.status()}>{(s) => <FlowStatusView s={s} c={c} />}</Show>
        </div>
    );
}

OAuthConnectPanel.displayName = "OAuthConnectPanel";
