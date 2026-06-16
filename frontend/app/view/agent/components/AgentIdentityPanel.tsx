// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentIdentityPanel — agent-scoped identity view.
 *
 * Shows which account each provider (GitHub, AWS, Anthropic, Custom) is
 * assigned to for this specific agent, with dropdowns to swap and an
 * unassign button. Replaces the global IdentityPanel in the agent card
 * settings panel as part of Step 4 of
 * SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md.
 *
 * The panel does NOT own account CRUD — creating a new account still goes
 * through the existing AccountForm via identityModel.openAddForm().
 */

import { createMemo, createSignal, For, onMount, Show, type JSX } from "solid-js";
import { AccountForm } from "@/app/view/identity/identity-view";
import { ProviderLogo } from "@/element/ProviderLogo";
import {
    type AccountProvider,
    type AgentAccounts,
    type IdentityViewModel,
    parseAgentAccounts,
    PROVIDER_LABELS,
    serializeAgentAccounts,
} from "@/app/view/identity/identity-model";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

const ALL_PROVIDERS: AccountProvider[] = ["github", "google", "aws", "openai", "anthropic", "slack", "custom"];

interface AgentIdentityPanelProps {
    agent: AgentDefinition;
    model: IdentityViewModel;
    /** Called whenever the user assigns or unassigns an account. The caller
     *  is responsible for persisting the new accounts value via RPC. */
    onUpdate: (newAccounts: AgentAccounts) => Promise<void>;
}

export const AgentIdentityPanel = (props: AgentIdentityPanelProps): JSX.Element => {
    const [saving, setSaving] = createSignal(false);
    const [saveError, setSaveError] = createSignal<string | null>(null);

    // Current assignments parsed from the agent record.
    const accounts = createMemo(() => parseAgentAccounts(props.agent));

    // For each provider, the Account object currently assigned (or null).
    const assignedAccount = (provider: AccountProvider) => {
        const id = accounts()[provider];
        if (!id) return null;
        return props.model.accountsAtom().find((a) => a.id === id) ?? null;
    };

    // Accounts available for each provider (for the dropdown).
    const accountsForProvider = (provider: AccountProvider) =>
        props.model.accountsAtom().filter((a) => a.provider === provider);

    const handleAssign = async (provider: AccountProvider, accountId: string | null) => {
        setSaveError(null);
        setSaving(true);
        try {
            const next: AgentAccounts = { ...accounts(), [provider]: accountId };
            await props.onUpdate(next);
        } catch (e) {
            setSaveError(String(e));
        } finally {
            setSaving(false);
        }
    };

    return (
        <div class="agent-identity-panel">
            <div class="agent-identity-header">
                <span class="agent-identity-title">
                    <span class="agent-identity-agent-name">{props.agent.name}</span>
                    {" "}uses the following accounts:
                </span>
            </div>

            <div class="agent-identity-providers">
                <For each={ALL_PROVIDERS}>
                    {(provider) => {
                        const assigned = () => assignedAccount(provider);
                        const options = () => accountsForProvider(provider);

                        return (
                            <div class="agent-identity-provider-row">
                                <ProviderLogo provider={provider} size={16} class="agent-identity-provider-icon" />
                                <span class="agent-identity-provider-label">
                                    {PROVIDER_LABELS[provider]}
                                </span>

                                <div class="agent-identity-provider-assignment">
                                    <Show
                                        when={assigned()}
                                        fallback={
                                            <span class="agent-identity-none">— none —</span>
                                        }
                                    >
                                        <span class="agent-identity-account-name">
                                            {assigned()!.display_name || assigned()!.name}
                                        </span>
                                        <Show when={assigned()!.context?.github_username}>
                                            <span class="agent-identity-username">
                                                @{assigned()!.context.github_username}
                                            </span>
                                        </Show>
                                        <Show when={assigned()!.context?.aws_profile}>
                                            <span class="agent-identity-username">
                                                {assigned()!.context.aws_profile}
                                            </span>
                                        </Show>
                                    </Show>
                                </div>

                                <div class="agent-identity-provider-actions">
                                    <Show when={options().length > 0}>
                                        <select
                                            class="agent-identity-select"
                                            disabled={saving()}
                                            value={accounts()[provider] ?? ""}
                                            onChange={(e) => {
                                                const v = e.currentTarget.value;
                                                void handleAssign(provider, v || null);
                                            }}
                                        >
                                            <option value="">— unassign —</option>
                                            <For each={options()}>
                                                {(acct) => (
                                                    <option value={acct.id}>
                                                        {acct.display_name || acct.name}
                                                    </option>
                                                )}
                                            </For>
                                        </select>
                                    </Show>
                                    <button
                                        class="agent-identity-new-btn"
                                        disabled={saving()}
                                        title={`Add new ${PROVIDER_LABELS[provider]} account`}
                                        onClick={() => {
                                            // Pre-populate provider in the form via a
                                            // temporary signal the AccountForm reads.
                                            // For now just open the generic add form.
                                            props.model.openAddForm();
                                        }}
                                    >
                                        + New
                                    </button>
                                    <Show when={assigned()}>
                                        <button
                                            class="agent-identity-unassign-btn"
                                            disabled={saving()}
                                            title="Unassign this account"
                                            onClick={() => void handleAssign(provider, null)}
                                        >
                                            ×
                                        </button>
                                    </Show>
                                </div>
                            </div>
                        );
                    }}
                </For>
            </div>

            <Show when={saveError()}>
                <div class="agent-identity-error">{saveError()}</div>
            </Show>

            {/* Inline account creation form — opened via "+ New" button */}
            <Show when={props.model.formOpenAtom()}>
                <AccountForm model={props.model} />
            </Show>

            {/* MuxBus cloud connectivity — global, not per-agent */}
            <MuxBusConnectSection />
        </div>
    );
};

AgentIdentityPanel.displayName = "AgentIdentityPanel";

/** Serialize AgentAccounts to the JSON string stored on AgentDefinition. */
export { serializeAgentAccounts };

// ── MuxBus Cloud Section ──────────────────────────────────────────────────────

// Production Cognito config — set after deployment.
// Override with VITE_MUXBUS_COGNITO_DOMAIN / VITE_MUXBUS_CLIENT_ID at build time.
const MUXBUS_COGNITO_DOMAIN =
    (import.meta.env.VITE_MUXBUS_COGNITO_DOMAIN as string | undefined) ??
    "https://muxbus-auth-prod.auth.us-east-1.amazoncognito.com";
const MUXBUS_CLIENT_ID =
    (import.meta.env.VITE_MUXBUS_CLIENT_ID as string | undefined) ?? "";

interface MuxBusStatus {
    connected: boolean;
    email: string;
    expiresAt: number;
    valid: boolean;
}

export const MuxBusConnectSection = (): JSX.Element => {
    const [status, setStatus] = createSignal<MuxBusStatus | null>(null);
    const [loading, setLoading] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const refreshStatus = async () => {
        try {
            const s = await RpcApi.MuxBusStatusCommand(TabRpcClient);
            setStatus(s);
        } catch {
            // no credentials or server not reachable — treat as disconnected
            setStatus({ connected: false, email: "", expiresAt: 0, valid: false });
        }
    };

    onMount(() => {
        void refreshStatus();
    });

    const handleConnect = async () => {
        if (!MUXBUS_CLIENT_ID) {
            setError("MuxBus client ID not configured (contact AgentMux team).");
            return;
        }
        setError(null);
        setLoading(true);
        try {
            const result = await RpcApi.MuxBusLoginCommand(TabRpcClient, {
                cognitoDomain: MUXBUS_COGNITO_DOMAIN,
                clientId: MUXBUS_CLIENT_ID,
            });
            if (result.success) {
                await refreshStatus();
            } else {
                setError(result.error ?? "Login failed.");
            }
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    const handleDisconnect = async () => {
        setError(null);
        setLoading(true);
        try {
            await RpcApi.MuxBusDisconnectCommand(TabRpcClient);
            await refreshStatus();
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    const expiryLabel = () => {
        const s = status();
        if (!s?.connected) return null;
        const exp = new Date(s.expiresAt * 1000);
        return exp.toLocaleString();
    };

    return (
        <div class="agent-identity-muxbus">
            <div class="agent-identity-section-title">AgentMux Cloud</div>
            <Show
                when={status()?.connected}
                fallback={
                    <div class="agent-identity-muxbus-row">
                        <span class="agent-identity-none">Not connected</span>
                        <button
                            class="agent-identity-new-btn"
                            disabled={loading()}
                            onClick={() => void handleConnect()}
                        >
                            {loading() ? "Connecting…" : "Connect"}
                        </button>
                    </div>
                }
            >
                <div class="agent-identity-muxbus-row">
                    <div class="agent-identity-muxbus-info">
                        <span class="agent-identity-account-name">{status()!.email}</span>
                        <Show when={!status()!.valid}>
                            <span class="agent-identity-muxbus-expired"> (token expired)</span>
                        </Show>
                        <Show when={expiryLabel()}>
                            <span class="agent-identity-muxbus-expiry"> · expires {expiryLabel()}</span>
                        </Show>
                    </div>
                    <button
                        class="agent-identity-unassign-btn"
                        disabled={loading()}
                        title="Disconnect from AgentMux Cloud"
                        onClick={() => void handleDisconnect()}
                    >
                        Disconnect
                    </button>
                </div>
            </Show>
            <Show when={error()}>
                <div class="agent-identity-error">{error()}</div>
            </Show>
        </div>
    );
};
