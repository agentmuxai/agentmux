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
/** How long after its section closes a connector that then connects (or gains an account) still counts as done from here. */
const CONNECT_GRACE_MS = 5000;

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
    /** The open connected card's Connect section, to add another account, is showing (§4.10). */
    adding: Accessor<boolean>;
    setAdding: (open: boolean) => void;
    close: () => void;
    toggle: (e: ConnectorEntry) => void;
    /** Connect: opens the card's Connect section, or closes it when it is already showing. */
    connect: (e: ConnectorEntry) => void;
    reconnect: (e: ConnectorEntry) => void;
    openCustom: () => void;
    /** The live entry for a connector key, for details that stay open while its accounts change. */
    entryOf: (key: string) => ConnectorEntry | undefined;
    /** `key` when it is open among `items`, else null: what a group's grid passes as its selected key. */
    openIn: (items: Accessor<ConnectorEntry[]>) => string | null;
}

/**
 * A connect succeeding has no success step to read and dismiss (§4.9): the Connect section closes and the card, now in
 * Connected, shows "Connected" once, in place. An account added to a card that was already connected (§4.10) does the same,
 * and leaves its details open. Only a card whose Connect section the person opened here counts, so the first load of the
 * list, where every account appears at once, flashes nothing. A Connect section that closes keeps its
 * place for a moment: saving the account closes the section, and the account reaches the list a beat later. After that a
 * connector that connects some other way (the agent pane's sign-in) does not flash here, since no connect was started in
 * this pane. Also closes the open card when it stops being connected (its last account was deleted), rather than its
 * Connect section opening by itself under it.
 */
/** How many accounts a card has, AgentMux Cloud's one session counting as one: 0 only before it is connected. */
const accountsOf = (e: ConnectorEntry) => (e.connected ? Math.max(e.accounts.length, 1) : 0);

function useConnectFlash(entries: Accessor<ConnectorEntry[]>, openKey: Accessor<string | null>, close: () => void) {
    // Per card: until when a connect counts, and how many accounts it had when its Connect section opened.
    const awaiting = new Map<string, { until: number; had: number }>();
    const [flash, setFlash] = createSignal<ReadonlySet<string>>(new Set());
    const timers: number[] = [];
    onCleanup(() => timers.forEach((id) => window.clearTimeout(id)));
    createEffect(() => {
        for (const e of entries()) {
            const wait = awaiting.get(e.key);
            if (wait === undefined || accountsOf(e) <= wait.had) continue;
            awaiting.delete(e.key);
            if (wait.until < Date.now()) continue;
            // The first account closes the card's Connect section; the card moves to Connected under it.
            if (wait.had === 0 && untrack(openKey) === e.key) close();
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
    createEffect(() => {
        const key = openKey();
        const e = key == null ? undefined : entries().find((x) => x.key === key);
        if (e && !e.connected && !untrack(() => awaiting.has(key!))) close();
    });
    return {
        flash,
        /** Its Connect section opened: a connect that follows is one the person started here. */
        waiting: (e: ConnectorEntry) => void awaiting.set(e.key, { until: Infinity, had: accountsOf(e) }),
        /** Its section closed (a key, or none): a connect within the grace period still counts. */
        parked: (key: string | null) => {
            const wait = key == null ? undefined : awaiting.get(key);
            if (key != null && wait) awaiting.set(key, { ...wait, until: Date.now() + CONNECT_GRACE_MS });
        },
    };
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
        flashes.parked(untrack(openKey));
        setOpenKey(null);
        setAdding(false);
        model.setSelectedAccount(null);
    };
    const flashes = useConnectFlash(entries, openKey, () => close());
    const open = (e: ConnectorEntry, withAdd = false) => {
        if (!e.connected || withAdd) flashes.waiting(e);
        model.setSelectedAccount(null);
        setAdding(withAdd);
        setOpenKey(e.key);
        // One account: its details show at once, not behind a second click, unless Connect opened the card.
        if (e.connected && e.accounts.length === 1 && !withAdd) model.setSelectedAccount(e.accounts[0]);
    };
    const setAddingOpen = (v: boolean) => {
        if (!v) flashes.parked(untrack(openKey));
        setAdding(v);
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
        flash: flashes.flash,
        adding,
        setAdding: setAddingOpen,
        close,
        toggle: (e) => (openKey() === e.key ? close() : open(e)),
        // A card that is not connected opens its Connect section as its details; a connected one, under its accounts.
        connect: (e) => {
            if (!e.connected) return openKey() === e.key ? close() : open(e);
            if (openKey() === e.key && adding()) setAddingOpen(false);
            else open(e, true);
        },
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
        entryOf: (key) => entries().find((e) => e.key === key),
        openIn: (items) => items().find((e) => e.key === openKey())?.key ?? null,
    };
}
