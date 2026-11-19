// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What Connectors' list keeps: the identity model, the AgentMux Cloud session, the cards (filtered, split into Connected
// and Available), and which card is open (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.5). One card is open at a
// time: its details when connected, its Connect section when not.

import { createEffect, createMemo, createSignal, onCleanup, onMount, untrack, type Accessor } from "solid-js";

import type { BlockNodeModel } from "@/app/block/blocktypes";
import { useAccountUseCounts } from "@/app/view/accounts/account-card";
import { useMuxBusStatus, type MuxBusController } from "@/app/view/accounts/AgentMuxConnectPanel";
import { requestReconnect } from "@/app/view/accounts/oauth-account-state";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { IdentityViewModel } from "@/app/view/identity/identity-model";
import { connectorEntries, connectorMatches, type ConnectorEntry } from "./connector-entries";

/** How long a card that has just connected shows "Connected" before it settles into its state. */
export const CONNECTED_FLASH_MS = 2800;

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
    /** The keys of cards that have just connected and are showing "Connected" (§4.9). */
    flash: Accessor<ReadonlySet<string>>;
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

    // A connect succeeding has no success step to read and dismiss (§4.9): the Connect section closes and the card,
    // now in Connected, shows "Connected" once, in place. Only a card whose Connect section the person opened here
    // counts, so the first load of the list, where every account appears at once, flashes nothing.
    const awaiting = new Set<string>();
    const [flash, setFlash] = createSignal<ReadonlySet<string>>(new Set());
    const timers: number[] = [];
    onCleanup(() => timers.forEach((id) => window.clearTimeout(id)));
    createEffect(() => {
        for (const e of entries()) {
            if (!e.connected || !awaiting.delete(e.key)) continue;
            if (untrack(openKey) === e.key) close();
            setFlash((s) => new Set(s).add(e.key));
            timers.push(
                window.setTimeout(
                    () =>
                        setFlash((s) => {
                            const n = new Set(s);
                            n.delete(e.key);
                            return n;
                        }),
                    CONNECTED_FLASH_MS
                )
            );
        }
    });
    const open = (e: ConnectorEntry, withAdd = false) => {
        if (!e.connected) awaiting.add(e.key);
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
        flash,
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
