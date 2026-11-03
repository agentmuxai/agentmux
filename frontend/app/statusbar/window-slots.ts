// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The instance panel's window list: each window, open or closed, in its own slot. A window is listed once, in
 * the place it first took; closing one greys its row in place and reopening it brings it back to the same
 * row. Only a window that is discarded leaves a gap, and the rows after it move up. A new window goes last.
 *
 * The one order is srv's (`session_slots.rs`, saved with the workspace, so it survives a restart and an
 * upgrade). A panel keeps none of its own: [`createWindowSlots`]'s `sync` sends the windows shown and takes
 * srv's order back, so every window's panel draws the same list. Until srv has answered, the panel draws
 * nothing in an order it would have to guess.
 */

import { createSignal } from "solid-js";

/** Far more windows than a workspace holds; only here so a long session cannot grow the list without end. */
const MAX_SLOTS = 200;

/**
 * `rows` in slot order: a row whose key already has a slot takes it, a new one goes last, in the order given.
 * `known` is the slots so far; the new slots come back with the rows.
 */
export function orderBySlots<T>(rows: readonly T[], keyOf: (row: T) => string, known: readonly string[]): { rows: T[]; slots: string[] } {
    const byKey = new Map<string, T>();
    for (const row of rows) {
        const key = keyOf(row);
        if (!byKey.has(key)) byKey.set(key, row);
    }
    const placed = new Set(known);
    const added = [...byKey.keys()].filter((key) => !placed.has(key));
    const next = [...known, ...added];
    // Past the cap, the oldest slots whose window is not listed now go first.
    let over = next.length - MAX_SLOTS;
    const kept = over > 0 ? next.filter((key) => byKey.has(key) || over-- <= 0) : next;
    return { rows: kept.filter((key) => byKey.has(key)).map((key) => byKey.get(key)!), slots: kept };
}

/** A key that stands in for a window until its workspace is known (never sent to srv). */
export const isStandIn = (key: string): boolean => key.startsWith("label:");

/** One panel's view of srv's order. */
export function createWindowSlots() {
    // srv's order as of its last answer, with the stand-in keys of windows whose workspace is not known yet.
    let slots: string[] = [];
    // The windows srv has a slot for, as of its last answer.
    let confirmed = new Set<string>();
    // Bumped when srv's answer changes the order, so what draws from `order` draws again.
    const [revision, setRevision] = createSignal(0);
    const [ready, setReady] = createSignal(false);
    return {
        /** Whether srv has answered (or failed to), so the order shown is its own. */
        ready,
        /** Whether srv keeps a slot for `key` (a window deleted anywhere has none). */
        has(key: string): boolean {
            revision();
            return slots.includes(key);
        },
        /** `rows` in slot order; a window with no slot yet goes last, as srv will place it. */
        order<T>(rows: readonly T[], keyOf: (row: T) => string): T[] {
            revision();
            const ordered = orderBySlots(rows, keyOf, slots);
            slots = ordered.slots;
            return ordered.rows;
        },
        /**
         * A window the host listed before its object had loaded has a slot under a stand-in key; once it is
         * known by its real key, that key takes the stand-in's slot (and nothing is done if it has one).
         */
        adopt(standIn: string, key: string): void {
            if (standIn === key || slots.includes(key)) return;
            const at = slots.indexOf(standIn);
            if (at >= 0) slots[at] = key;
        },
        /** Whether srv has not yet been told about one of `keys` (the panel then asks again). */
        needsSync(keys: readonly string[]): boolean {
            return keys.some((k) => !confirmed.has(k));
        },
        /**
         * Tell srv which windows are shown (`place`: `WindowService.PlaceWindowSlots`) and take its order:
         * the ones it has no slot for go at the end, and a window deleted anywhere has left a gap that is
         * closed. Only the stand-ins of windows not known yet are kept beside it.
         */
        async sync(keys: readonly string[], place: (keys: string[]) => Promise<string[]>): Promise<void> {
            const real = keys.filter((k) => !isStandIn(k));
            if (real.length === 0) return;
            try {
                const server = (await place(real)) ?? [];
                const known = new Set(server);
                const next = [...server, ...slots.filter((k) => !known.has(k) && isStandIn(k))];
                const changed = next.length !== slots.length || next.some((k, i) => k !== slots[i]);
                slots = next;
                confirmed = known;
                if (changed) setRevision((r) => r + 1);
            } finally {
                setReady(true);
            }
        },
    };
}

/** What a closed window's row says: its name and how many of its agents are running without it, "Wizzy · 4 agents running". */
export function closedWindowLabel(name: string, running: number): string {
    const shown = name.trim() || "Window";
    if (running <= 0) return shown;
    return `${shown} · ${running} ${running === 1 ? "agent" : "agents"} running`;
}
