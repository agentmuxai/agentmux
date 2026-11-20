// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What opens under a connected connector's card (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.5): its accounts,
// the "instances", each with its own details; the Connect section its card's Connect opens, to add another account
// (§4.10); and, for AgentMux Cloud, its session. Nothing here is new: the account cards and their details are the ones
// Connectors' Accounts section showed, moved under the connector.

import { Show, type Accessor, type JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { CardGrid, openMenuAt } from "@/app/element/ui";
import { AccountCard } from "@/app/view/accounts/account-card";
import { AddAccountSection } from "@/app/view/accounts/AddAccountSection";
import { AgentMuxConnectBody, type MuxBusController } from "@/app/view/accounts/AgentMuxConnectPanel";
import { requestReconnect } from "@/app/view/accounts/oauth-account-state";
import { failoverToggle } from "@/app/view/identity/account-failover";
import { buildAccountRowMenu } from "@/app/view/identity/bind-to-agent-menu";
import { AccountDetailPanel } from "@/app/view/identity/identity-accounts-tab";
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
                <Accounts {...props} />
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
            </div>
        </Show>
    );
}
