// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AccountsManager: the Accounts section of the Connectors pane. Brand chips to
// add an account on top, then a card for everything connected
// (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.5; first built for
// docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md).
//
// It owns a block-free `IdentityViewModel` (account CRUD goes through the
// module-level RPC + cache, so the blockId/nodeModel it carries for the
// ViewModel interface are inert here). Opening a card shows the same
// `AccountDetailPanel` the agent pane's Identity tab shows under a row.
//
// AgentMux Cloud is a "virtual first-class account": a single app-wide
// session (the `muxbus.*` singleton), NOT a stored IdentityAccount. It gets a
// card of its own that opens the connect panel, where Disconnect lives.

import { createMemo, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { BlockNodeModel } from "@/app/block/blocktypes";
import { Button, CardGrid, FilterInput } from "@/app/element/ui";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { buildAccountRowMenu } from "@/app/view/identity/bind-to-agent-menu";
import { AccountForm } from "@/app/view/identity/identity-account-form";
import {
    AccountDeleteNotice,
    AccountDetailPanel,
    DeleteAccountConfirm,
} from "@/app/view/identity/identity-accounts-tab";
import { IdentityViewModel, type Account } from "@/app/view/identity/identity-model";
import { matchesEveryWord } from "@/app/util/fuzzysearch";
import { AccountCard, accountSearchText, AgentMuxCard, sortAccounts, useAccountUseCounts } from "./account-card";
import { AccountsGallery } from "./AccountsGallery";
import { AgentMuxConnectPanel, useMuxBusStatus } from "./AgentMuxConnectPanel";
import { ClaudeLoginPanel } from "./ClaudeLoginPanel";
import "@/app/view/identity/identity-view.scss";

type ConnectedItem = { kind: "agentmux" } | { kind: "account"; account: Account };

const AGENTMUX_KEY = "agentmux-cloud";

export function AccountsManager(): JSX.Element {
    // Block-free instance: a synthetic id and a stub nodeModel are safe, since
    // all account CRUD flows through the module-level cache and RPCs.
    const model = new IdentityViewModel("connectors:accounts", {} as BlockNodeModel);
    onCleanup(() => model.dispose());

    const muxbus = useMuxBusStatus();
    onMount(() => void muxbus.refresh());
    const agentMuxConnected = () => muxbus.status()?.connected === true;

    const [agents] = useAgentDefinitions();
    const usedBy = useAccountUseCounts(() => agents());

    const [muxPanelOpen, setMuxPanelOpen] = createSignal(false);
    const [claudePanelOpen, setClaudePanelOpen] = createSignal(false);
    const [filter, setFilter] = createSignal("");

    const connected = createMemo<ConnectedItem[]>(() => {
        const q = filter();
        const items: ConnectedItem[] = [];
        if (agentMuxConnected() && matchesEveryWord(q, muxbus.status()?.email, "AgentMux Cloud")) {
            items.push({ kind: "agentmux" });
        }
        for (const account of sortAccounts(model.accountsAtom())) {
            if (matchesEveryWord(q, ...accountSearchText(account))) items.push({ kind: "account", account });
        }
        return items;
    });
    const total = () => model.accountsAtom().length + (agentMuxConnected() ? 1 : 0);

    // Right-click a card: Bind to agent ▸, Copy account ID, Delete account….
    // Latest right-click wins: the menu awaits an RPC, and an earlier one
    // resolving late must not open for the wrong account.
    let menuSeq = 0;
    const openMenu = (account: Account, e: MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        const seq = ++menuSeq;
        void buildAccountRowMenu(
            account,
            agents(),
            model.accountsAtom(),
            (msg) => model.showNotice(msg),
            undefined,
            () => model.requestDelete(account),
        ).then((items) => {
            if (seq === menuSeq) ContextMenuModel.showContextMenu(items, e);
        });
    };

    const toggle = (account: Account) =>
        model.setSelectedAccount(model.selectedAccountAtom()?.id === account.id ? null : account);

    return (
        <div class="identity-view accounts-manager">
            <div class="identity-body accounts-manager-body">
                <section class="ui-card-section ui-density-compact" aria-label="Accounts">
                    <div class="ui-card-section-header">
                        <span class="ui-card-section-title">Accounts</span>
                        <Show when={total() > 0}>
                            <span class="ui-card-section-count">{total()}</span>
                        </Show>
                        <FilterInput
                            class="ui-card-section-filter"
                            placeholder="Filter accounts"
                            value={filter()}
                            onInput={setFilter}
                        />
                        <Button tone="accent" icon="plus" onClick={() => model.openAddForm()}>
                            Add account
                        </Button>
                    </div>

                    {/* Brand chips: pick a service → OAuth / Key, or AgentMux. */}
                    <AccountsGallery
                        model={model}
                        agentMuxConnected={agentMuxConnected}
                        onAgentMux={() => setMuxPanelOpen(true)}
                        onClaudeConnect={() => setClaudePanelOpen(true)}
                    />

                    <AccountDeleteNotice model={model} />

                    <h3 class="ui-card-group-heading accounts-connected-heading">Connected</h3>
                    <Show
                        when={connected().length > 0}
                        fallback={
                            <div class="ui-card-empty">
                                {total() > 0
                                    ? `No accounts match “${filter()}”.`
                                    : "No accounts connected yet. Pick a service above to add one."}
                            </div>
                        }
                    >
                        <CardGrid
                            items={connected()}
                            ariaLabel="Connected accounts"
                            keyOf={(item) => (item.kind === "agentmux" ? AGENTMUX_KEY : item.account.id)}
                            selectedKey={model.selectedAccountAtom()?.id ?? null}
                            onClose={() => model.setSelectedAccount(null)}
                            renderCard={(item) =>
                                item.kind === "agentmux" ? (
                                    <AgentMuxCard status={muxbus.status()} onSelect={() => setMuxPanelOpen(true)} />
                                ) : (
                                    <AccountCard
                                        account={item.account}
                                        usedBy={usedBy().get(item.account.id) ?? 0}
                                        selected={model.selectedAccountAtom()?.id === item.account.id}
                                        onSelect={() => toggle(item.account)}
                                        onContextMenu={(e) => openMenu(item.account, e)}
                                    />
                                )
                            }
                            renderDetail={(item) =>
                                item.kind === "account" ? (
                                    // The selected account, not the grid's copy: an
                                    // update from elsewhere shows in place.
                                    <Show when={model.selectedAccountAtom()} keyed>
                                        {(selected) => <AccountDetailPanel model={model} account={selected} />}
                                    </Show>
                                ) : null
                            }
                        />
                    </Show>
                </section>
            </div>

            <Show when={model.pendingDeleteAtom()} keyed>
                {(account) => <DeleteAccountConfirm model={model} account={account} />}
            </Show>

            <Show when={model.formOpenAtom()}>
                <AccountForm model={model} />
            </Show>

            <Show when={muxPanelOpen()}>
                <AgentMuxConnectPanel muxbus={muxbus} onClose={() => setMuxPanelOpen(false)} />
            </Show>

            <Show when={claudePanelOpen()}>
                {/* Connectors has no ModalLayer/PaneModalScope ancestor: "tab"
                    resolves via tabcontent.tsx's outer ModalLayer scope="tab". */}
                <ClaudeLoginPanel onClose={() => setClaudePanelOpen(false)} scope="tab" />
            </Show>
        </div>
    );
}

AccountsManager.displayName = "AccountsManager";
