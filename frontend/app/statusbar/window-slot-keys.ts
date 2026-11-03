// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The instance panel's window rows, keyed and put in slot order: each window, open or closed, by its
 * workspace, in the place srv keeps for it (`window-slots.ts`).
 */

import { createEffect, createMemo, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { getObjectValue, makeORef } from "@/store/mos";
import { WindowService, type ClosedWindow } from "@/store/services";
import { createWindowSlots, isStandIn } from "./window-slots";

/**
 * How long a window that has just closed keeps its row while srv has not yet listed it as closed. The host
 * reports a close before srv has recorded it (the window's own renderer tells srv as it goes), so for a moment
 * the window is in neither list; srv's record normally comes within a second or two.
 */
export const CLOSING_HOLD_MS = 10_000;

/** A window that was open a moment ago and is not listed yet as closed: its row stays, greyed. */
export interface HeldRow {
    key: string;
    name: string;
    since: number;
}

/**
 * The rows held after a change. `wasOpen` is the name each window had on the last look; a window in it that
 * is now neither `open` nor `closed` is held (once). A held row ends when the window is listed again, open or
 * closed, or when `keep` says srv no longer has its slot (the window was deleted anywhere), or after
 * [`CLOSING_HOLD_MS`]. A window with nothing worth keeping is deleted at its close and is not listed: its row
 * goes away once, at the end of the hold or sooner, and does not come back.
 */
export function nextHeld(
    wasOpen: ReadonlyMap<string, string>,
    open: ReadonlySet<string>,
    closed: ReadonlySet<string>,
    held: readonly HeldRow[],
    now: number
): HeldRow[] {
    const kept = held.filter((h) => !open.has(h.key) && !closed.has(h.key) && now - h.since < CLOSING_HOLD_MS);
    const have = new Set(kept.map((h) => h.key));
    for (const [key, name] of wasOpen) {
        if (isStandIn(key) || open.has(key) || closed.has(key) || have.has(key)) continue;
        kept.push({ key, name, since: now });
    }
    return kept.length === held.length && kept.every((h, i) => h === held[i]) ? (held as HeldRow[]) : kept;
}

const HELD = Symbol("held");

const heldWindows = new WeakMap<HeldRow, ClosedWindow>();

/** A held row as the closed window it is about to be listed as, so it draws as one. The same object each time, so the row is not rebuilt. */
export function heldWindow(h: HeldRow): ClosedWindow {
    let w = heldWindows.get(h);
    if (!w) {
        w = Object.assign({ workspaceid: h.key, closed_at_ms: h.since, name: h.name, tabs: 0, panes: 0, agents: [], running: 0 } as ClosedWindow, { [HELD]: true });
        heldWindows.set(h, w);
    }
    return w;
}

/** Whether `w` is a held row, not one srv has listed. */
export const isHeldWindow = (w: ClosedWindow): boolean => (w as unknown as Record<symbol, unknown>)[HELD] === true;

/** What the host says of an open window. */
export interface SlotWindowEntry {
    label: string;
    windowId: string | null;
}

export function useSlotKeys(
    entries: Accessor<SlotWindowEntry[]>,
    closed: Accessor<ClosedWindow[]>,
    windowIdOf: (entry: SlotWindowEntry) => string | null,
    nameOf: (entry: SlotWindowEntry, index: number) => string
) {
    const slots = createWindowSlots();
    // The window a row stands for, by its workspace (a closed window is listed by it). Until the host has
    // said which window it is, by its label; the slot it took then moves to its workspace once that is known.
    const standInKey = (entry: SlotWindowEntry): string => `label:${entry.label}`;
    const keyOfEntry = (entry: SlotWindowEntry): string => {
        const windowId = windowIdOf(entry);
        const win = windowId ? getObjectValue<MuxWindow>(makeORef("window", windowId)) : null;
        if (!win?.workspaceid) return standInKey(entry);
        slots.adopt(standInKey(entry), win.workspaceid);
        return win.workspaceid;
    };
    const openByKey = createMemo(() => new Map(entries().map((e) => [keyOfEntry(e), e] as const)));
    const closedByKey = createMemo(() => new Map(closed().map((w) => [w.workspaceid, w] as const)));
    const shownKeys = (): string[] => [...openByKey().keys(), ...closedByKey().keys()].filter((k) => !isStandIn(k));
    // A window that has just closed keeps its row, as a closed one, until srv lists it: the host reports the
    // close before srv has recorded it, and a row that went and came back is a flicker.
    const [held, setHeld] = createSignal<HeldRow[]>([]);
    let wasOpen = new Map<string, string>();
    // Windows this panel is deleting (the row's own Remove): they leave for good, so no row is held for them.
    const discarding = new Map<string, number>();
    const discardedNow = (key: string, now: number): boolean => (discarding.get(key) ?? -Infinity) + CLOSING_HOLD_MS > now;
    createEffect(() => {
        const open = openByKey();
        const closedNow = closedByKey();
        const names = new Map([...open].map(([key, entry]) => [key, nameOf(entry, Math.max(entries().indexOf(entry), 0))] as const));
        untrack(() => {
            const now = Date.now();
            const leaving = new Map([...wasOpen].filter(([key]) => !discardedNow(key, now)));
            setHeld((h) => nextHeld(leaving, new Set(open.keys()), new Set(closedNow.keys()), h, now));
        });
        wasOpen = names;
    });
    // The hold ends by itself, so a window that srv never lists (nothing in it worth keeping) does not stay.
    createEffect(() => {
        const rows = held();
        if (rows.length === 0) return;
        const next = Math.min(...rows.map((h) => h.since + CLOSING_HOLD_MS)) - Date.now();
        const timer = setTimeout(() => setHeld((h) => nextHeld(new Map(), new Set(openByKey().keys()), new Set(closedByKey().keys()), h, Date.now())), Math.max(next, 0) + 1);
        onCleanup(() => clearTimeout(timer));
    });
    // The held rows srv still has a slot for: one it deleted (anywhere) is gone for good.
    const heldByKey = createMemo(() => new Map(held().filter((h) => slots.has(h.key)).map((h) => [h.key, h] as const)));
    // The closed windows to draw: srv's, and the held ones as closed windows (not yet listed, so not reopened).
    const closedRows = createMemo(
        () => new Map([...[...heldByKey()].filter(([k]) => !openByKey().has(k)).map(([k, h]) => [k, heldWindow(h)] as const), ...closedByKey()])
    );
    // Every window, open or closed, in srv's order: nothing until srv has said it, so no panel draws a guess.
    const slotKeys = createMemo(() =>
        !slots.ready() && shownKeys().length > 0
            ? []
            : slots.order([...openByKey().keys(), ...closedByKey().keys(), ...heldByKey().keys()], (k) => k)
    );
    // Tell srv the windows shown (it gives a new one the next slot at the end) and follow what it says.
    const placeSlots = (): void => {
        const keys = shownKeys();
        if (keys.length === 0) return;
        slots.sync(keys, (k) => WindowService.PlaceWindowSlots(k)).catch((e) =>
            console.warn("[InstancePanel] couldn't place the window slots:", e)
        );
    };
    createEffect(() => {
        if (slots.needsSync(shownKeys())) untrack(placeSlots);
    });
    /** The window of `entry` is being deleted: its row goes with it, not held. */
    const discard = (entry: SlotWindowEntry): void => {
        discarding.set(keyOfEntry(entry), Date.now());
    };
    return { openByKey, closedByKey: closedRows, slotKeys, placeSlots, discard };
}
