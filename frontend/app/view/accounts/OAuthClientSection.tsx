// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which OAuth client a connect runs with (D8 and §7 of
// docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md): the
// client saved in the keychain, or the BYO fields once, with the console
// steps. srv never sends the secret back; it lives only in its input here.

import { createSignal, For, Show, type Accessor, type JSX } from "solid-js";

import { Button, Pill, SegmentedControl } from "@/app/element/ui";
import { openLink } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AccountOAuthClientInfo } from "@/types/rpc/AccountOAuthClientInfo";
import { clientModeLabel } from "./oauth-account-state";
import { byoSecretRequired, needsByo, type OAuthServiceInfo } from "./oauth-catalog";

type ClientMode = "byo" | "workspace";

/** The client part of an `account.oauth.start` request. */
export interface OAuthClientFields {
    clientId?: string;
    clientSecret?: string;
    clientMode?: ClientMode;
}

export interface OAuthClientState {
    /** srv's answer is in (or failed): the panel knows what to show. */
    loaded: Accessor<boolean>;
    info: Accessor<AccountOAuthClientInfo | null>;
    /** The BYO fields are showing. */
    showFields: Accessor<boolean>;
    /** The client fields for the request, or the sentence that's missing. */
    requestFields: () => { fields: OAuthClientFields } | { error: string };
    /** Empty the secret input, once the flow has it. */
    clearSecret: () => void;
}

/**
 * Reads the saved client once and keeps the BYO inputs. `clientIdHint`
 * prefills the client id (a reconnect whose saved client was forgotten).
 */
export function createOAuthClientState(svc: OAuthServiceInfo, clientIdHint?: string) {
    const [info, setInfo] = createSignal<AccountOAuthClientInfo | null>(null);
    const [loaded, setLoaded] = createSignal(false);
    const [changing, setChanging] = createSignal(false);
    const [clientId, setClientId] = createSignal(clientIdHint ?? "");
    const [clientSecret, setClientSecret] = createSignal("");
    const [mode, setMode] = createSignal<ClientMode>("byo");
    const [forgetting, setForgetting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const load = async () => {
        try {
            setInfo(await RpcApi.AccountOAuthClientGetCommand(TabRpcClient, { provider: svc.provider }));
        } catch {
            // Unknown: the fields show, and a connect with them still works.
            setInfo(null);
        } finally {
            setLoaded(true);
        }
    };
    void load();

    const saved = () => info()?.saved === true;
    const showFields = () => loaded() && (changing() || (!saved() && needsByo(svc, info()?.builtinAvailable)));

    const forget = async () => {
        setForgetting(true);
        setError(null);
        try {
            await RpcApi.AccountOAuthClientClearCommand(TabRpcClient, { provider: svc.provider });
            setChanging(false);
            await load();
        } catch (e) {
            setError(`Couldn't forget the client: ${(e as Error)?.message ?? String(e)}`);
        } finally {
            setForgetting(false);
        }
    };

    const requestFields = (): { fields: OAuthClientFields } | { error: string } => {
        if (!showFields()) return { fields: {} };
        const id = clientId().trim();
        const secret = clientSecret().trim();
        if (!id) return { error: "Client ID is required" };
        if (byoSecretRequired(svc) && !secret) return { error: "Client secret is required" };
        return {
            fields: {
                clientId: id,
                clientSecret: secret || undefined,
                clientMode: svc.workspaceMode ? mode() : undefined,
            },
        };
    };

    const state: OAuthClientState = {
        loaded,
        info,
        showFields,
        requestFields,
        clearSecret: () => setClientSecret(""),
    };
    const view = {
        svc,
        saved,
        changing,
        setChanging,
        clientId,
        setClientId,
        clientSecret,
        setClientSecret,
        mode,
        setMode,
        forgetting,
        forget,
        error,
    };
    return { state, view };
}

type ClientView = ReturnType<typeof createOAuthClientState>["view"];

export function OAuthClientSection(props: { client: ReturnType<typeof createOAuthClientState> }): JSX.Element {
    const s = props.client.state;
    const v: ClientView = props.client.view;
    return (
        <Show when={s.loaded()} fallback={<div class="oauth-waiting">Checking for a saved client…</div>}>
            <Show when={v.error()}>
                <div class="identity-form-error">{v.error()}</div>
            </Show>
            <Show when={v.saved() && !v.changing()}>
                <div class="oauth-saved-client">
                    <span>Using your saved client</span>
                    <code class="oauth-client-id">{s.info()?.clientId}</code>
                    <Pill>{clientModeLabel(s.info()?.mode)}</Pill>
                </div>
                <div class="identity-key-actions">
                    <Button density="compact" onClick={() => v.setChanging(true)}>
                        Use a different client
                    </Button>
                    <Button density="compact" tone="danger" busy={v.forgetting()} onClick={() => void v.forget()}>
                        Forget this client
                    </Button>
                </div>
            </Show>
            <Show when={s.showFields()}>
                <ByoFields view={v} />
            </Show>
        </Show>
    );
}

function ByoFields(props: { view: ClientView }): JSX.Element {
    const v = props.view;
    const svc = v.svc;
    return (
        <>
            <div class="oauth-byo-note">
                <Show
                    when={svc.consoleSteps}
                    fallback={<>ⓘ {svc.byoHint ?? "Supply your own OAuth app's client id."}</>}
                >
                    {(steps) => (
                        <ol class="oauth-steps">
                            <For each={steps()}>{(step) => <li>{step}</li>}</For>
                        </ol>
                    )}
                </Show>
                <Show when={svc.consoleUrl}>
                    {" "}
                    <a
                        class="oauth-link"
                        href="#"
                        onClick={(e) => {
                            e.preventDefault();
                            openLink(svc.consoleUrl!);
                        }}
                    >
                        Open developer console ↗
                    </a>
                </Show>
            </div>
            <Show when={svc.workspaceMode}>
                <div class="identity-form-field">
                    <label class="identity-form-label">Client type</label>
                    <SegmentedControl<ClientMode>
                        ariaLabel="Client type"
                        density="compact"
                        value={v.mode()}
                        onChange={v.setMode}
                        options={[
                            { value: "byo", label: "Your own client" },
                            { value: "workspace", label: "Workspace internal" },
                        ]}
                    />
                </div>
                <Show when={v.mode() === "workspace"}>
                    <div class="oauth-waiting">
                        An Internal app of your Workspace organization: no Google review, for people in your
                        organization only.
                    </div>
                </Show>
            </Show>
            <div class="identity-form-field">
                <label class="identity-form-label">Client ID</label>
                <input
                    class="identity-input"
                    type="text"
                    value={v.clientId()}
                    onInput={(e) => v.setClientId(e.currentTarget.value)}
                    placeholder={svc.provider === "google" ? "1234-abc.apps.googleusercontent.com" : "Iv1.0123456789abcdef"}
                />
            </div>
            <Show when={byoSecretRequired(svc)}>
                <div class="identity-form-field">
                    <label class="identity-form-label">Client secret</label>
                    <input
                        class="identity-input"
                        type="password"
                        autocomplete="off"
                        value={v.clientSecret()}
                        onInput={(e) => v.setClientSecret(e.currentTarget.value)}
                        placeholder="stored in your OS keychain — never in plaintext"
                    />
                </div>
            </Show>
            <Show when={v.saved()}>
                <div class="identity-key-actions">
                    <Button density="compact" onClick={() => v.setChanging(false)}>
                        Use the saved client
                    </Button>
                </div>
            </Show>
        </>
    );
}
