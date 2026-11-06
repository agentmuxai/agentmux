// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The OAuth connect panel's markup in parts (OAuthConnectPanel.tsx): the idle row (the client and Connect, or
// the Cloud sign-in), the running flow (device code, browser prompt, error, confirm, Cancel) and a flow's error.

import { Show, type Accessor, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { openLink } from "@/app/store/global";
import type { OAuthFlowStatus } from "@/app/store/rpc-api";
import { writeText } from "@/util/clipboard";
import { isNeedsCloudSignInError } from "./github-cloud-gate";
import { GithubCloudSignIn } from "./GithubCloudSignIn";
import { isBlockedError } from "./oauth-account-state";
import type { OAuthConnectPanelProps, OAuthConnectState } from "./oauth-connect-state";
import { ConnectRow } from "./OAuthClientSection";

/** A flow's error; an organization's block gets its own heading (spec §7). */
export function FlowError(props: { error: string }): JSX.Element {
    // srv asking for the AgentMux Cloud sign-in is a question with an answer, not an error.
    if (isNeedsCloudSignInError(props.error)) return <GithubCloudSignIn />;
    return (
        <div class="identity-form-error" role="alert">
            <Show when={isBlockedError(props.error)}>
                <div class="oauth-error-title">Blocked by your organization</div>
            </Show>
            {props.error}
        </div>
    );
}

/** Idle: the client + Connect, or, for GitHub without the Cloud sign-in, the sign-in in place of Connect. */
export function IdleRow(props: { c: OAuthConnectState; panel: OAuthConnectPanelProps }): JSX.Element {
    const { c, panel } = props;
    return (
        <Show
            when={!c.needsCloud()}
            fallback={
                <>
                    <GithubCloudSignIn />
                    <Show when={panel.onCancel}>
                        <div class="identity-key-actions">
                            <Button class="identity-btn identity-btn-secondary" onClick={() => panel.onCancel?.()}>
                                Cancel
                            </Button>
                        </div>
                    </Show>
                </>
            }
        >
            <ConnectRow
                client={c.client}
                unavailable={c.unavailable()}
                busy={c.busy()}
                label={c.label}
                reconnect={!!panel.accountId}
                onConnect={() => void c.connect()}
                onCancel={panel.onCancel}
            />
        </Show>
    );
}

/** The device code, with Copy and the verification page. */
function DeviceCode(props: { s: Accessor<OAuthFlowStatus> }): JSX.Element {
    const s = props.s;
    return (
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
    );
}

/** Waiting for the browser, with a link to open it again while its address is known. */
function BrowserWaiting(props: { s: Accessor<OAuthFlowStatus> }): JSX.Element {
    const s = props.s;
    return (
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
    );
}

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

/** Running: the device code or browser prompt, a failure, GitHub's confirm, and Cancel (Try again after a failure). */
export function FlowStatusView(props: { s: Accessor<OAuthFlowStatus>; c: OAuthConnectState }): JSX.Element {
    const { s, c } = props;
    return (
        <div class="oauth-flow-status">
            <Show when={s().status === "code-emitted"}>
                <DeviceCode s={s} />
            </Show>

            <Show when={s().status === "pending" || s().status === "url-available"}>
                <BrowserWaiting s={s} />
            </Show>

            <Show when={s().status === "failed"}>
                <FlowError error={(s() as { error: string }).error} />
            </Show>

            <Show when={s().status === "confirm-user"}>
                <ConfirmUser
                    status={s() as Extract<OAuthFlowStatus, { status: "confirm-user" }>}
                    onReplace={c.replaceUser}
                    onKeep={c.reset}
                />
            </Show>

            <Show when={s().status !== "confirm-user"}>
                <div class="identity-key-actions">
                    <Show
                        when={s().status === "failed"}
                        fallback={
                            <Button class="identity-btn identity-btn-secondary" onClick={c.reset}>
                                Cancel
                            </Button>
                        }
                    >
                        <Button class="identity-btn identity-btn-secondary" onClick={c.reset}>
                            Try again
                        </Button>
                    </Show>
                </div>
            </Show>
        </div>
    );
}
