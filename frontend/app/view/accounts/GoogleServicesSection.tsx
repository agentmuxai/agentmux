// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Google account's services in its details: each one's state and Turn on
// (an incremental consent for just its scopes), the agents bound to the
// account with their level on it, and what agents did with it
// (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §7, D7).

import { createResource, createSignal, For, Show, type JSX } from "solid-js";

import { Button, SegmentedControl } from "@/app/element/ui";
import type { Account } from "@/app/view/identity/identity-model";
import {
    callLine,
    canManageGrants,
    loadGooglePanel,
    serviceState,
    SERVICES,
    setGoogleGrant,
    type GooglePanel,
    type GoogleService,
    type PanelAgent,
    type ServiceInfo,
} from "./google-services";
import { OAuthConnectPanel } from "./OAuthConnectPanel";

/** Log lines shown before "Show all". */
const CALLS_SHOWN = 10;

export function GoogleServicesSection(props: { account: Account }): JSX.Element {
    const a = () => props.account;
    // The fetcher never rejects: a rejected resource throws on every read.
    // A failed load is data here, shown in place of the lists.
    const [loaded, { refetch }] = createResource(
        () => (canManageGrants() ? a().id : null),
        (id): Promise<{ panel: GooglePanel | null; error: string | null }> =>
            loadGooglePanel(id).then(
                (panel) => ({ panel, error: null }),
                (e) => ({ panel: null, error: e instanceof Error ? e.message : String(e) })
            )
    );
    const panel = () => loaded()?.panel ?? null;
    const loadError = () => loaded()?.error ?? null;
    const [error, setError] = createSignal<string | null>(null);
    const [allCalls, setAllCalls] = createSignal(false);

    const setLevel = async (agent: PanelAgent, service: GoogleService, level: "read" | "off") => {
        setError(null);
        try {
            await setGoogleGrant(a().id, agent.agentId, service, level);
            await refetch();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    const calls = () => panel()?.calls ?? [];
    const agents = () => panel()?.agents ?? [];

    return (
        <>
            <div class="identity-detail-section">Services</div>
            <For each={SERVICES}>{(info) => <ServiceRow account={a()} info={info} />}</For>

            <div class="identity-detail-section">Agents</div>
            <Show
                when={canManageGrants()}
                fallback={<div class="identity-detail-hint">Open AgentMux's desktop app to choose which agents use this account.</div>}
            >
                <Show
                    when={agents().length > 0}
                    fallback={
                        <Show when={panel()}>
                            <div class="identity-detail-hint">
                                No agent is bound to this account. Bind one to an agent, then choose what it may use here.
                            </div>
                        </Show>
                    }
                >
                    <For each={agents()}>
                        {(agent) => (
                            <For each={SERVICES}>
                                {(info) => (
                                    <div class="identity-detail-field">
                                        <span class="identity-detail-label">
                                            {agent.name || agent.agentId}: {info.label}
                                        </span>
                                        <SegmentedControl
                                            density="compact"
                                            ariaLabel={`${info.label} for ${agent.name || agent.agentId}`}
                                            options={[
                                                { value: "off", label: "Off" },
                                                { value: "read", label: "Read" },
                                            ]}
                                            value={agent.levels[info.service] ? "read" : "off"}
                                            onChange={(v) => void setLevel(agent, info.service, v)}
                                        />
                                    </div>
                                )}
                            </For>
                        )}
                    </For>
                </Show>
                <Show when={error() ?? loadError()}>
                    {(e) => (
                        <div class="identity-form-error" role="alert">
                            {e()}
                        </div>
                    )}
                </Show>

                <div class="identity-detail-section">Activity</div>
                <Show when={calls().length > 0} fallback={<div class="identity-detail-hint">No agent has used it yet.</div>}>
                    <For each={allCalls() ? calls() : calls().slice(0, CALLS_SHOWN)}>
                        {(c) => (
                            <div class="identity-detail-field">
                                <span class="identity-detail-label">{new Date(c.at).toLocaleString()}</span>
                                <span class="identity-detail-value">
                                    {callLine(c)}
                                    {c.outcome === "ok" ? "" : `: ${c.outcome}`}
                                </span>
                            </div>
                        )}
                    </For>
                    <Show when={!allCalls() && calls().length > CALLS_SHOWN}>
                        <div class="identity-key-actions">
                            <Button density="compact" onClick={() => setAllCalls(true)}>
                                Show all {calls().length}
                            </Button>
                        </div>
                    </Show>
                </Show>
            </Show>
        </>
    );
}

function ServiceRow(props: { account: Account; info: ServiceInfo }): JSX.Element {
    const [turningOn, setTurningOn] = createSignal(false);
    const state = () => serviceState(props.info, props.account.context.granted_scopes ?? []);
    const stateText = () => ({ on: "On", partly: "Partly granted", off: "Off" })[state()];
    return (
        <>
            <div class="identity-detail-field">
                <span class="identity-detail-label">{props.info.label}</span>
                <span class="identity-detail-value">{stateText()}</span>
            </div>
            <div class="identity-detail-hint">{props.info.line}</div>
            <Show
                when={turningOn()}
                fallback={
                    <Show when={state() !== "on"}>
                        <div class="identity-key-actions">
                            <Button density="compact" tone="accent" onClick={() => setTurningOn(true)}>
                                {state() === "off" ? `Turn on ${props.info.label}` : `Ask again for ${props.info.label}`}
                            </Button>
                        </div>
                    </Show>
                }
            >
                <OAuthConnectPanel
                    provider={props.account.provider}
                    accountId={props.account.id}
                    scopes={props.info.readScopes}
                    clientIdHint={props.account.context.oauth_client?.client_id}
                    autoStart
                    onConnected={() => setTurningOn(false)}
                    onCancel={() => setTurningOn(false)}
                />
            </Show>
        </>
    );
}
