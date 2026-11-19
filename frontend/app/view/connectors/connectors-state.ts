// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What Connectors' list keeps: the identity model, the AgentMux Cloud session, the cards (filtered, split into Connected
// and Available), and which card is open (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.5). One card is open at a
// time: its details when connected, its Connect section when not.

import { createMemo, createSignal, onCleanup, onMount, type Accessor } from "solid-js";

import type { BlockNodeModel } from "@/app/block/blocktypes";
import { useAccountUseCounts } from "@/app/view/accounts/account-card";
import { useMuxBusStatus, type MuxBusController } from "@/app/view/accounts/AgentMuxConnectPanel";
import { requestReconnect } from "@/app/view/accounts/oauth-account-state";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { IdentityViewModel } from "@/app/view/identity/identity-model";
import { connectorEntries, connectorMatches, type ConnectorEntry } from "./connector-entries";

export interface ConnectorsState {
    model: IdentityViewModel;
    muxbus: MuxBusController;
    usedBy: Accessor<Map<string, number>>;
    filter: Accessor<string>;
    setFilter: (q: string) => void;
    /** The cards that match the filter, Connected and Available. */
    connected: Accessor<ConnectorEntry[]>;
    available: Accessor<ConnectorEntry[]>;
    shownCount: Accessor<number>;
    connectedCount: Accessor<number>;
    openKey: Accessor<string | null>;
    /** The open connected card's Add another account section is showing. */
    adding: Accessor<boolean>;
    setAdding: (open: boolean) => void;
    close: () => void;
    toggle: (e: ConnectorEntry) => void;
    reconnect: (e: ConnectorEntry) => void;
    openCustom: () => void;
    /** `key` when it is open among `items`, else null: what a group's grid passes as its selected key. */
    openIn: (items: Accessor<ConnectorEntry[]>) => string | null;
}

export function useConnectorsState(): ConnectorsState {
    // Block-free: account CRUD goes through the module-level RPC and cache, so the blockId and nodeModel are inert.
    const model = new IdentityViewModel("connectors:accounts", {} as BlockNodeModel);
    onCleanup(() => model.dispose());

    const muxbus = useMuxBusStatus();
    onMount(() => void muxbus.refresh());
    const [agents] = useAgentDefinitions();
    const usedBy = useAccountUseCounts(() => agents());

    const [filter, setFilter] = createSignal("");
    const [openKey, setOpenKey] = createSignal<string | null>(null);
    const [adding, setAdding] = createSignal(false);

    const entries = createMemo(() => connectorEntries(model.accountsAtom(), muxbus.status()));
    const shown = createMemo(() => entries().filter((e) => connectorMatches(e, filter(), muxbus.status()?.email)));
    const connected = () => shown().filter((e) => e.connected);
    const available = () => shown().filter((e) => !e.connected);

    const close = () => {
        setOpenKey(null);
        setAdding(false);
        model.setSelectedAccount(null);
    };
    const open = (e: ConnectorEntry, withAdd = false) => {
        model.setSelectedAccount(null);
        setAdding(withAdd);
        setOpenKey(e.key);
        // One account: its details show at once, not behind a second click.
        if (e.connected && e.accounts.length === 1) model.setSelectedAccount(e.accounts[0]);
    };
    return {
        model,
        muxbus,
        usedBy,
        filter,
        setFilter,
        connected,
        available,
        shownCount: () => shown().length,
        connectedCount: () => entries().filter((e) => e.connected).length,
        openKey,
        adding,
        setAdding,
        close,
        toggle: (e) => (openKey() === e.key ? close() : open(e)),
        // Reconnect runs in the account's details, which show the browser prompt.
        reconnect: (e) => {
            if (e.reconnect) requestReconnect(e.reconnect.id);
            open(e);
            if (e.reconnect) model.setSelectedAccount(e.reconnect);
        },
        openCustom: () => {
            setFilter("");
            const custom = entries().find((e) => e.key === "custom");
            if (custom) open(custom, custom.connected);
        },
        openIn: (items) => items().find((e) => e.key === openKey())?.key ?? null,
    };
}
