// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The instance panel's window rows, keyed and put in slot order: each window, open or closed, by its
 * workspace, in the place srv keeps for it (`window-slots.ts`).
 */

import { createEffect, createMemo, untrack, type Accessor } from "solid-js";
import { getObjectValue, makeORef } from "@/store/mos";
import { WindowService, type ClosedWindow } from "@/store/services";
import { createWindowSlots, isStandIn } from "./window-slots";

/** What the host says of an open window. */
export interface SlotWindowEntry {
    label: string;
    windowId: string | null;
}

export function useSlotKeys(
    entries: Accessor<SlotWindowEntry[]>,
    closed: Accessor<ClosedWindow[]>,
    windowIdOf: (entry: SlotWindowEntry) => string | null
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
    // Every window, open or closed, in srv's order: nothing until srv has said it, so no panel draws a guess.
    const slotKeys = createMemo(() =>
        !slots.ready() && shownKeys().length > 0
            ? []
            : slots.order([...openByKey().keys(), ...closedByKey().keys()], (k) => k)
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
    return { openByKey, closedByKey, slotKeys, placeSlots };
}
