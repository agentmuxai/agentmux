// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The instance panel's window list keeps each window's slot. A window is listed once, open or closed, in the
 * place it first took; closing one greys its row in place and reopening it brings it back to the same row.
 * Only a window that is discarded leaves a gap, and the rows after it move up.
 *
 * The slot order lives here, not in the panel, so it survives the panel being closed and opened again within
 * a session. The first time a set of windows is seen they go in the order given (open windows as the host
 * lists them, then closed ones newest first); a window met later goes after the ones already placed.
 */

/** The slots in this renderer, oldest first. A window that is gone keeps its slot until the cap, so one that
 *  is only between lists for a moment (closed, not yet in srv's closed list) does not lose its place. */
let slots: string[] = [];

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

/** [`orderBySlots`] on this renderer's slots, which it records. Same rows in, same order out. */
export function inSlotOrder<T>(rows: readonly T[], keyOf: (row: T) => string): T[] {
    const ordered = orderBySlots(rows, keyOf, slots);
    slots = ordered.slots;
    return ordered.rows;
}

/**
 * A window the host listed before its object had loaded has a slot under a stand-in key; once it is known by
 * its real key, that key takes the stand-in's slot (and nothing is done if the real key has a slot already).
 */
export function adoptSlot(standIn: string, key: string): void {
    if (standIn === key || slots.includes(key)) return;
    const at = slots.indexOf(standIn);
    if (at >= 0) slots[at] = key;
}

/** Forget every slot (tests). */
export function resetWindowSlots(): void {
    slots = [];
}

/** What a closed window's row says: its name and how many of its agents are running without it, "Wizzy · 4 agents running". */
export function closedWindowLabel(name: string, running: number): string {
    const shown = name.trim() || "Window";
    if (running <= 0) return shown;
    return `${shown} · ${running} ${running === 1 ? "agent" : "agents"} running`;
}
