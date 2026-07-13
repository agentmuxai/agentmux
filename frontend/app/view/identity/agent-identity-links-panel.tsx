// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AgentIdentityLinksPanel — read-only Provider/Account/Status table for
// ONE agent's direct account links (`db_agent_identity_links`, the table
// the spawn-time resolver actually reads —
// agentmux-srv/src/identity/resolver.rs). Given an `agentId`, this is the
// same row shape the now-removed Armory "Identities" rail tab
// (`AgentIdentitiesPanel`) rendered per selected agent — extracted here so
// the agent-pane's own `view: "identity"` tab (`identity-pane-view.tsx`)
// can render it directly for its own block's agent, closing the data gap
// documented in docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md
// §1.3.
//
// No create/edit/delete/bind/unbind here — new agent identities are
// created from the agent-launch flow directly. See
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

interface AgentIdentityLinksPanelProps {
    /** The agent whose linked accounts to show. `undefined` when this
     *  panel has no agent context (e.g. a generically opened identity
     *  block) — renders a context-free empty state in that case. */
    agentId: string | undefined;
}

export const AgentIdentityLinksPanel = (props: AgentIdentityLinksPanelProps): JSX.Element => {
    const agents = useAgentDefinitions();

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

    // Re-subscribe to this agent's own link-change event whenever the
    // agentId prop changes — mirrors AgentLaunchModal.tsx's
    // `identitybundlebindings:changed:<id>` pattern.
    createEffect(() => {
        const id = props.agentId;
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

    const agent = createMemo<AgentDefinition | null>(() => {
        const id = props.agentId;
        if (!id) return null;
        return agents().find((a) => a.id === id) ?? null;
    });

    const rows = createMemo(() => {
        const id = props.agentId;
        if (!id) return [];
        return joinAgentIdentityRows(id, allLinks(), accountsById());
    });

    return (
        <div class="identity-pane">
            <div class="identity-pane-detail">
                <Show when={error()}>
                    <div class="identity-pane-error">{error()}</div>
                </Show>

                <Show
                    when={props.agentId}
                    fallback={
                        <div class="identity-pane-empty">
                            <p>This pane isn't attached to a specific agent.</p>
                            <p class="identity-pane-empty-hint">
                                Open the Identity tab from an agent's own pane to see the accounts
                                that agent launches with.
                            </p>
                        </div>
                    }
                >
                    <div class="identity-pane-readonly">
                        <h2 class="identity-pane-name">{agent()?.name ?? "This agent"}</h2>

                        <h3 class="identity-pane-section-title">Linked accounts</h3>
                        <Show
                            when={rows().length > 0}
                            fallback={
                                <p class="identity-pane-empty-hint">
                                    This agent has no linked accounts yet. Link one from its launch
                                    dialog.
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
                                                        {(PROVIDER_LABELS as Record<string, string>)[
                                                            row.provider
                                                        ] ?? row.provider}
                                                    </td>
                                                    <td>
                                                        {row.account
                                                            ? row.account.display_name?.trim() ||
                                                              row.account.name
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
                </Show>
            </div>
        </div>
    );
};

AgentIdentityLinksPanel.displayName = "AgentIdentityLinksPanel";
