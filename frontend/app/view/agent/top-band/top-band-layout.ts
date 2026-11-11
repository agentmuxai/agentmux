// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's top band: which of its items are shown in which form, from their real measured widths.
 * Pure functions, no DOM; the band (AgentTopBand.tsx) measures and renders, this decides. The rule is the one in
 * SPEC_AGENT_PANE_CPU_MEMORY_READOUT_IN_COMPOSER_STRIP_2026_11_07.md section 2.2:
 *
 *   1. Everything in its full form, if it fits.
 *   2. If not, the lowest-priority item that has a compact form goes compact, then the next lowest, until it fits.
 *      Controls are the target and keep their full size.
 *   3. If everything that can be compact is compact and it still does not fit, the lowest-priority item with no
 *      compact form is dropped (never a control), and the layout says so.
 *   4. No item is drawn over another: each gets its own box, in order.
 */

/** The band's fixed slots, left to right. */
export type BandSlot = "identity" | "stats" | "controls";

export const BAND_SLOTS: readonly BandSlot[] = ["identity", "stats", "controls"];

export type BandForm = "full" | "compact";

/** What the layout needs to know of one item. Widths are in the same px as the band's width. */
export interface BandItemSize {
    id: string;
    slot: BandSlot;
    /** Higher keeps its full form longer. */
    priority: number;
    /** Registration order: of two items with the same priority, the earlier one keeps its form longer. */
    order: number;
    /** The width it wants in its full form (an elastic item, such as the find bar, says the least it can do with). */
    full: number;
    /** The width of its compact form; absent when it has none. */
    compact?: number;
}

export interface BandLayout {
    forms: Record<string, BandForm>;
    /** Items the band leaves out, lowest priority first. Never a control. */
    dropped: string[];
    /** Whether everything shown fits in the width. False only when a control or a compact item is alone too wide. */
    fits: boolean;
}

/** Lowest priority first; of equal priority, the later registered first (the earlier keeps its form longer). */
const giveUpOrder = (a: BandItemSize, b: BandItemSize): number => a.priority - b.priority || b.order - a.order;

export function layoutTopBand(width: number, items: readonly BandItemSize[], gap: number): BandLayout {
    const forms: Record<string, BandForm> = {};
    const dropped = new Set<string>();
    for (const item of items) forms[item.id] = "full";

    const total = (): number => {
        let sum = 0;
        let shown = 0;
        for (const item of items) {
            if (dropped.has(item.id)) continue;
            sum += forms[item.id] === "compact" ? (item.compact ?? item.full) : item.full;
            shown++;
        }
        return shown === 0 ? 0 : sum + gap * (shown - 1);
    };
    const fits = (): boolean => total() <= width;

    if (!fits()) {
        const shrinkable = items
            .filter((i) => i.slot !== "controls" && i.compact !== undefined && i.compact < i.full)
            .sort(giveUpOrder);
        for (const item of shrinkable) {
            forms[item.id] = "compact";
            if (fits()) break;
        }
    }
    if (!fits()) {
        const droppable = items.filter((i) => i.slot !== "controls" && i.compact === undefined).sort(giveUpOrder);
        for (const item of droppable) {
            dropped.add(item.id);
            if (fits()) break;
        }
    }
    return { forms, dropped: items.filter((i) => dropped.has(i.id)).sort(giveUpOrder).map((i) => i.id), fits: fits() };
}
