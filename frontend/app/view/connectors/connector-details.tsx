// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What opens under a connected connector's card (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.5): its accounts,
// the "instances", each with its own details; the Connect section its card's Connect opens, to add another account
// (§4.10); and, for AgentMux Cloud, its session. Nothing here is new: the account cards and their details are the ones
// Connectors' Accounts section showed, moved under the connector.

import { createEffect, onCleanup, Show, untrack, type Accessor, type JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { CardGrid, openMenuAt } from "@/app/element/ui";
import { AccountCard } from "@/app/view/accounts/account-card";
import { AddAccountSection } from "@/app/view/accounts/AddAccountSection";
import { AgentMuxConnectBody, type MuxBusController } from "@/app/view/accounts/AgentMuxConnectPanel";
import { requestReconnect } from "@/app/view/accounts/oauth-account-state";
import { failoverToggle } from "@/app/view/identity/account-failover";
import { buildAccountRowMenu } from "@/app/view/identity/bind-to-agent-menu";
import { AccountDetailPanel, DeleteAccountPanel } from "@/app/view/identity/identity-accounts-tab";
import type { Account, IdentityViewModel } from "@/app/view/identity/identity-model";
import type { ConnectorEntry } from "./connector-entries";

export interface ConnectorDetailsProps {
    /** The connector, read live: its accounts change while its details are open (an account added, deleted or updated). */
    entry: Accessor<ConnectorEntry>;
    model: IdentityViewModel;
    muxbus: MuxBusController;
    /** How many agents use each account. */
    usedBy: Accessor<Map<string, number>>;
    /** The Connect section, to add another account, is open. */
    adding: Accessor<boolean>;
    setAdding: (open: boolean) => void;
}

/** One account of the connector: its card, with the details under the selected one. */
function Accounts(props: ConnectorDetailsProps): JSX.Element {
    const { model } = props;
    const toggle = (a: Account) => model.setSelectedAccount(model.selectedAccountAtom()?.id === a.id ? null : a);
    const openMenu = (a: Account, e: MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        openMenuAt(buildAccountRowMenu(a, () => model.requestDelete(a), failoverToggle(a)), e);
    };
    // The delete question hangs under the account it is about, if that account is one of this connector's.
    const asked = () => {
        const pending = model.pendingDeleteAtom();
        return pending && props.entry().accounts.some((a) => a.id === pending.id) ? pending.id : null;
    };
    // The question goes with its row: an account that is gone (deleted elsewhere), or the card closing, answers it with
    // nothing deleted.
    createEffect(() => {
        const pending = model.pendingDeleteAtom();
        if (pending && !model.accountsAtom().some((a) => a.id === pending.id)) model.cancelDelete();
    });
    onCleanup(() => {
        const pending = untrack(model.pendingDeleteAtom);
        if (pending && untrack(() => props.entry().accounts).some((a) => a.id === pending.id)) model.cancelDelete();
    });
    return (
        <CardGrid
            class="connector-accounts"
            ariaLabel={t("connectors.accountsLabel", { name: props.entry().name })}
            items={props.entry().accounts}
            keyOf={(a) => a.id}
            // One column however wide: an account's line is read as a list.
            minCardWidth={100000}
            selectedKey={model.selectedAccountAtom()?.id ?? null}
            onClose={() => model.setSelectedAccount(null)}
            askKey={asked()}
            renderAsk={() => (
                // The pending account itself: what confirming deletes, and the label it asks about.
                <Show when={model.pendingDeleteAtom()} keyed>
                    {(pending) => <DeleteAccountPanel model={model} account={pending} />}
                </Show>
            )}
            renderCard={(a) => (
                <AccountCard
                    account={a}
                    all={props.entry().accounts}
                    usedBy={props.usedBy().get(a.id) ?? 0}
                    selected={model.selectedAccountAtom()?.id === a.id}
                    onSelect={() => toggle(a)}
                    onContextMenu={(e) => openMenu(a, e)}
                    onReconnect={() => {
                        requestReconnect(a.id);
                        model.setSelectedAccount(a);
                    }}
                />
            )}
            renderDetail={() => (
                // The selected account, not the grid's copy: an update from elsewhere shows in place.
                <Show when={model.selectedAccountAtom()} keyed>
                    {(selected) => <AccountDetailPanel model={model} account={selected} />}
                </Show>
            )}
        />
    );
}

export function ConnectorDetails(props: ConnectorDetailsProps): JSX.Element {
    return (
        <Show
            when={props.entry().tile.id !== "agentmux"}
            fallback={<AgentMuxConnectBody muxbus={props.muxbus} />}
        >
            <div class="connector-details">
                {/* Above the accounts: under a long list it opened out of sight, and the press on Connect looked dead. */}
                <Show when={props.adding()}>
                    <AddAccountSection
                        tile={props.entry().tile}
                        name={props.entry().name}
                        service={props.entry().service}
                        model={props.model}
                        muxbus={props.muxbus}
                        onClose={() => props.setAdding(false)}
                    />
                </Show>
                <Accounts {...props} />
            </div>
        </Show>
    );
}
