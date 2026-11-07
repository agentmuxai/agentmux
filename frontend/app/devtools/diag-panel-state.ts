// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { dispatchRecordsAtom, describeSource, type DispatchRecord } from "@/store/command-source";
import { keyCommands } from "@/app/store/keymodel-dispatch";

const DEFAULT_DISPLAY_LIMIT = 80;

export interface DisplayRow {
    /** Index into the live records array — used for stable keys. */
    idx: number;
    /** Original record's wall-clock timestamp. The `age` string is
     *  computed inline in the render so it doesn't pollute the
     *  `rows` memo with a time dependency — see the createMemo
     *  comment for why that matters. */
    at: number;
    slice: string;
    /** Per-pane key, truncated to 7 chars when present. */
    keyShort: string;
    /** Command discriminator (e.g. "Navigate", "PaneClicked"). */
    cmdType: string;
    source: string;
    /** Number of events emitted by the dispatch. */
    eventCount: number;
}

function commandType(cmd: unknown): string {
    if (cmd != null && typeof cmd === "object" && "type" in cmd) {
        const t = (cmd as { type?: unknown }).type;
        if (typeof t === "string") return t;
    }
    return "?";
}

export function ageMs(now: number, at: number): string {
    const dt = now - at;
    if (dt < 1000) return `${dt}ms ago`;
    if (dt < 60_000) return `${(dt / 1000).toFixed(1)}s ago`;
    return `${(dt / 60_000).toFixed(1)}min ago`;
}

function toRow(idx: number, rec: DispatchRecord): DisplayRow {
    return {
        idx,
        at: rec.at,
        slice: rec.slice,
        keyShort: rec.key ? rec.key.slice(0, 7) : "—",
        cmdType: commandType(rec.command),
        source: describeSource(rec.source),
        eventCount: rec.events.length,
    };
}

/** The panel's state: whether it shows, its filters, the records they leave and the clock the ages follow. */
export function createDiagPanel() {
    const [visible, setVisible] = createSignal(false);
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));
    const [sliceFilter, setSliceFilter] = createSignal<string>("");
    const [keyFilter, setKeyFilter] = createSignal<string>("");

    // Its key is a shortcut-table row (`dev:diagnostics`), so it can't collide
    // with another binding and also works from a terminal.
    onMount(() => {
        keyCommands.set("dev:diagnostics", () => {
            setVisible((v) => !v);
            return true;
        });
    });
    onCleanup(() => keyCommands.delete("dev:diagnostics"));

    // Available slices for the filter dropdown — derived from the
    // current records. Recomputed reactively so a new slice that
    // shows up appears in the dropdown without a manual refresh.
    const availableSlices = createMemo(() => {
        const seen = new Set<string>();
        for (const r of dispatchRecordsAtom()) seen.add(r.slice);
        return Array.from(seen).sort();
    });

    // Filtered + sliced view, newest first. Limited to
    // DEFAULT_DISPLAY_LIMIT rows so a runaway dispatch (which the
    // audit ring is precisely there to catch) doesn't wedge the
    // panel itself.
    //
    // **Important:** this memo deliberately does NOT depend on `now()`.
    // If it did, the 1 Hz age-tick interval would force a rebuild of
    // every DisplayRow and `<For>` would re-create every <tr> on
    // each tick — exactly the kind of inefficiency the panel exists
    // to surface. The `age` cell reads `now()` inline in the render
    // (Solid's fine-grained reactivity isolates the update to that
    // single text node).
    const rows = createMemo<DisplayRow[]>(() => {
        const all = dispatchRecordsAtom();
        const slice = sliceFilter();
        const key = keyFilter().trim().toLowerCase();
        const filtered: DisplayRow[] = [];
        // Walk newest-to-oldest so we can stop once we've collected
        // the display limit — saves work on large rings.
        for (let i = all.length - 1; i >= 0 && filtered.length < DEFAULT_DISPLAY_LIMIT; i--) {
            const r = all[i];
            if (slice !== "" && r.slice !== slice) continue;
            if (key !== "") {
                const recKey = (r.key ?? "").toLowerCase();
                if (!recKey.includes(key)) continue;
            }
            filtered.push(toRow(i, r));
        }
        return filtered;
    });

    const totalDispatches = createMemo(() => dispatchRecordsAtom().length);

    return { visible, now, sliceFilter, setSliceFilter, keyFilter, setKeyFilter, availableSlices, rows, totalDispatches };
}

export type DiagPanelState = ReturnType<typeof createDiagPanel>;
