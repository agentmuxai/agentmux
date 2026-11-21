// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createMemo, createSignal, untrack, type Accessor } from "solid-js";
import type { WindowEntry } from "@/store/global";

/**
 * The open windows minus the ones this panel has just removed, by label: the row goes at once, not when the host
 * has finished tearing the window down (on macOS that took 8 to 13 s, the row meanwhile reading as a live window
 * with no opacity slider, as the host drops its id first). A label leaves the set once the host no longer lists it.
 * `hide(label, false)` puts a row back (the close failed).
 */
export function useTrashedWindows(entries: Accessor<WindowEntry[]>): {
    visibleEntries: Accessor<WindowEntry[]>;
    hide: (label: string, on: boolean) => void;
} {
    const [trashed, setTrashed] = createSignal<ReadonlySet<string>>(new Set());
    const visibleEntries = createMemo(() => entries().filter((e) => !trashed().has(e.label)));
    createEffect(() => {
        const listed = new Set(entries().map((e) => e.label));
        const t = untrack(trashed);
        if ([...t].some((l) => !listed.has(l))) setTrashed(new Set([...t].filter((l) => listed.has(l))));
    });
    const hide = (label: string, on: boolean) =>
        setTrashed((t) => new Set(on ? [...t, label] : [...t].filter((l) => l !== label)));
    return { visibleEntries, hide };
}
