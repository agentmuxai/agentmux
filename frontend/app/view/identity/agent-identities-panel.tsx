// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AgentIdentitiesPanel — read-only, per-agent view of direct agent↔account
// links (`db_agent_identity_links`), the table the spawn-time resolver
// actually reads (agentmux-srv/src/identity/resolver.rs). Replaces the
// Armory "Identities" rail tab's old full bundle-CRUD UI
// (`identity-manager.tsx`'s former `IdentityManager`, deleted as dead
// code — its two intended mount points never had a live caller). The
// agent-pane's own `view: "identity"` settings tab is a separate,
// already-read-only surface (`<BundleSummaryPanel/>`) and is untouched.
//
// No create/edit/delete/bind/unbind here — new agent identities are now
// created from the agent-launch flow directly (issue #1624 PR-C). This
// panel only shows what's already linked. See
// docs/specs/SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md.

import { createEffect, createMemo, createSignal, For, onCleanup, Show, type JSX } from "solid-js";

import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { loadAccounts, subscribeAccountChanges, PROVIDER_LABELS, type Account } from "./identity-model";
import { statusBadge } from "./identity-manager";
import { joinAgentIdentityRows } from "./agent-identities-model";

import "./identity-pane-view.scss";

export const AgentIdentitiesPanel = (): JSX.Element => {
    const agents = useAgentDefinitions();

    const [selectedId, setSelectedId] = createSignal<string | null>(null);
    const [allLinks, setAllLinks] = createSignal<AgentDefinitionIdentity[]>([]);
    const [accounts, setAccounts] = createSignal<Account[]>(loadAccounts());
    const [error, setError] = createSignal<string | null>(null);

    const refreshLinks = async (): Promise<void> => {
        try {
            const result = await RpcApi.ListAllAgentIdentitiesCommand(TabRpcClient);
            setAllLinks(result ?? []);
            setError(null);
        } catch (e: any) {
            setError(e?.message ?? "Failed to load agent identities");
        }
    };
    void refreshLinks();

    const unsubAccounts = subscribeAccountChanges(setAccounts);
    onCleanup(unsubAccounts);

    // Re-subscribe to the selected agent's own link-change event whenever
    // selection changes — mirrors AgentLaunchModal.tsx's
    // `identitybundlebindings:changed:<id>` pattern. The event name embeds
    // the agent id, so a subscription only ever tracks one agent at a time.
    createEffect(() => {
        const id = selectedId();
        if (!id) return;
        const unsub = waveEventSubscribe({
            eventType: `agentidentities:changed:${id}`,
            handler: () => void refreshLinks(),
        });
        onCleanup(unsub);
    });

    const accountsById = createMemo<Map<string, Account>>(() => {
        const m = new Map<string, Account>();
        for (const a of accounts()) m.set(a.id, a);
        return m;
    });

    const selectedAgent = createMemo<AgentDefinition | null>(() => {
        const id = selectedId();
        if (!id) return null;
        return agents().find((a) => a.id === id) ?? null;
    });

    const rows = createMemo(() => {
        const id = selectedId();
        if (!id) return [];
        return joinAgentIdentityRows(id, allLinks(), accountsById());
    });

    return (
        <div class="identity-pane">
            <div class="identity-pane-rail">
                <div class="identity-pane-rail-header">
                    <span class="identity-pane-list-item-desc">Agents</span>
                </div>
                <ul class="identity-pane-list">
                    <For each={agents()}>
                        {(agent) => (
                            <li
                                class="identity-pane-list-item"
                                classList={{ "is-selected": selectedId() === agent.id }}
                                onClick={() => setSelectedId(agent.id)}
                            >
                                <div class="identity-pane-list-item-name">{agent.name}</div>
                            </li>
                        )}
                    </For>
                </ul>
            </div>

            <div class="identity-pane-detail">
                <Show when={error()}>
                    <div class="identity-pane-error">{error()}</div>
                </Show>

                <Show
                    when={selectedAgent()}
                    fallback={
                        <div class="identity-pane-empty">
                            <p>Select an agent from the list to see its linked accounts.</p>
                            <p class="identity-pane-empty-hint">
                                This is a read-only view of the direct account links each agent
                                actually launches with. To link a new account, pick or create one
                                from the agent's launch dialog.
                            </p>
                        </div>
                    }
                >
                    {(agent) => (
                        <div class="identity-pane-readonly">
                            <h2 class="identity-pane-name">{agent().name}</h2>

                            <h3 class="identity-pane-section-title">Linked accounts</h3>
                            <Show
                                when={rows().length > 0}
                                fallback={
                                    <p class="identity-pane-empty-hint">
                                        This agent has no linked accounts yet. Link one from its
                                        launch dialog.
                                    </p>
                                }
                            >
                                <table class="identity-pane-bindings">
                                    <thead>
                                        <tr>
                                            <th>Provider</th>
                                            <th>Account</th>
                                            <th>Status</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        <For each={rows()}>
                                            {(row) => {
                                                const badge = createMemo(() => statusBadge(row.account?.status));
                                                return (
                                                    <tr>
                                                        <td>
                                                            {(PROVIDER_LABELS as Record<string, string>)[row.provider] ??
                                                                row.provider}
                                                        </td>
                                                        <td>
                                                            {row.account
                                                                ? row.account.display_name?.trim() || row.account.name
                                                                : "—"}
                                                        </td>
                                                        <td>
                                                            <span class="identity-pane-status">
                                                                <span
                                                                    class={`identity-pane-status-dot is-${badge().dot}`}
                                                                    aria-hidden="true"
                                                                />
                                                                <span class="identity-pane-status-label">
                                                                    {badge().label}
                                                                </span>
                                                            </span>
                                                        </td>
                                                    </tr>
                                                );
                                            }}
                                        </For>
                                    </tbody>
                                </table>
                            </Show>
                        </div>
                    )}
                </Show>
            </div>
        </div>
    );
};
