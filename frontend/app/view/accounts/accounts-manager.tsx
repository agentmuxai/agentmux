// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AccountsManager — the context-free Accounts management UI, rendered as the
// **Accounts** tab of the Armory (bundle-manager-modal). Phase 1 of
// specs/archive/SPEC_TRUST_CENTER_2026_06_15.md: surface the existing IdentityAccount model
// app-wide with no backend changes.
//
// It owns a block-free `IdentityViewModel` instance (the existing accounts
// view-model — account CRUD goes through the module-level RPC + cache, so the
// blockId/nodeModel it carries for the ViewModel interface are inert here) and
// composes the already-built `AccountsTab` + `AccountForm` so styling and the
// add/edit/delete lifecycle come for free. Identity-bundle and Memory
// management remain in their own Armory tabs.
//
// AgentMux Cloud is also surfaced here as a "virtual first-class account": it
// is a single app-wide session (the `muxbus.*` singleton), NOT a pluralizable
// IdentityAccount, so it is not stored in `db_accounts`. Instead this
// tab projects the live `muxbus.status` into the gallery tile count and a
// read-only connected row, and drives connect/disconnect through the shared
// `useMuxBusStatus` controller. Scoped to this tab only — the per-agent
// identity panel is unaffected.

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { BlockNodeModel } from "@/app/block/blocktypes";
import { IdentityViewModel } from "@/app/view/identity/identity-model";
import { AccountsTab } from "@/app/view/identity/identity-accounts-tab";
import { AccountForm } from "@/app/view/identity/identity-account-form";
import { ProviderLogo } from "@/element/ProviderLogo";
import { AccountsGallery } from "./AccountsGallery";
import { AgentMuxConnectPanel, useMuxBusStatus } from "./AgentMuxConnectPanel";
import { ClaudeLoginPanel } from "./ClaudeLoginPanel";
import "@/app/view/identity/identity-view.scss";

export function AccountsManager(): JSX.Element {
    // Block-free instance. `IdentityViewModel` only uses blockId/nodeModel to
    // satisfy the ViewModel interface; all account CRUD flows through the
    // module-level cache + `*IdentityAccountCommand` RPCs, so a synthetic id
    // and a stub nodeModel are safe. Created once per mount (SolidJS component
    // bodies run a single time).
    const model = new IdentityViewModel("armory:accounts", {} as BlockNodeModel);
    onCleanup(() => model.dispose());

    // Shared AgentMux Cloud session controller — single source of truth for the
    // tile count, the read-only row, and the connect panel.
    const muxbus = useMuxBusStatus();
    onMount(() => void muxbus.refresh());

    const [muxPanelOpen, setMuxPanelOpen] = createSignal(false);
    const [claudePanelOpen, setClaudePanelOpen] = createSignal(false);
    const agentMuxConnected = () => muxbus.status()?.connected === true;

    const muxStatusDot = (): string => {
        const s = muxbus.status();
        if (s?.connected && s.valid) return "status-dot status-valid";
        if (s?.connected) return "status-dot status-expired";
        return "status-dot status-unknown";
    };

    return (
        <div class="identity-view">
            <div class="identity-header">
                <span class="identity-header-title">Accounts</span>
                <button
                    class="identity-add-btn"
                    onClick={() => model.openAddForm()}
                    title="Add account"
                >
                    + Add
                </button>
            </div>

            <div class="identity-body accounts-manager-body">
                {/* Brand-tile landing: pick a service → OAuth / Key, or AgentMux. */}
                <AccountsGallery
                    model={model}
                    agentMuxConnected={agentMuxConnected}
                    onAgentMux={() => setMuxPanelOpen(true)}
                    onClaudeConnect={() => setClaudePanelOpen(true)}
                />
                {/* Connected accounts (manage existing). Shown when there is any
                    stored account OR an AgentMux Cloud session is connected.
                    Flows at natural height below the gallery — .identity-body
                    (the shared parent) is the single scroll boundary for the
                    whole tab, not this section on its own. */}
                <Show when={model.accountsAtom().length > 0 || agentMuxConnected()}>
                    <div class="accounts-manager-list-section">
                        <div class="accounts-connected-heading">Connected accounts</div>
                        {/* AgentMux Cloud — read-only projected row (singleton
                            session, not a stored IdentityAccount). Clicking it opens
                            the connect panel, where Disconnect lives. */}
                        <Show when={agentMuxConnected()}>
                            <div class="identity-accounts-list">
                                <div class="identity-group">
                                    <div class="identity-group-header">AgentMux</div>
                                    <div
                                        class="identity-account-row"
                                        onClick={() => setMuxPanelOpen(true)}
                                        title="Manage AgentMux Cloud connection"
                                    >
                                        <span class="identity-provider-badge provider-agentmux">
                                            <ProviderLogo provider="agentmux" size={16} />
                                        </span>
                                        <span class="identity-account-name">
                                            {muxbus.status()?.email || "AgentMux Cloud"}
                                        </span>
                                        <div class="identity-row-meta">
                                            <span class="identity-display-name">
                                                AgentMux Cloud
                                                <Show when={!muxbus.status()?.valid}> · token expired</Show>
                                            </span>
                                        </div>
                                        <span
                                            class={muxStatusDot()}
                                            title={muxbus.status()?.valid ? "valid" : "expired"}
                                        />
                                    </div>
                                </div>
                            </div>
                        </Show>
                        <Show when={model.accountsAtom().length > 0}>
                            <AccountsTab model={model} />
                        </Show>
                    </div>
                </Show>
            </div>

            <Show when={model.formOpenAtom()}>
                <AccountForm model={model} />
            </Show>

            <Show when={muxPanelOpen()}>
                <AgentMuxConnectPanel muxbus={muxbus} onClose={() => setMuxPanelOpen(false)} />
            </Show>

            <Show when={claudePanelOpen()}>
                {/* Armory has no ModalLayer/PaneModalScope ancestor — "tab"
                    resolves via tabcontent.tsx's outer ModalLayer scope="tab". */}
                <ClaudeLoginPanel onClose={() => setClaudePanelOpen(false)} scope="tab" />
            </Show>
        </div>
    );
}

AccountsManager.displayName = "AccountsManager";
