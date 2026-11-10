// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent pane puts in its top band. An item says which slot it is for, how long it keeps its full form
 * (priority), and how to draw each form; the band lays every item out by one rule (top-band-layout.ts), so a
 * control added later gets the rule with no layout code of its own.
 */

import { createSignal, type Accessor, type JSX } from "solid-js";
import type { BandSlot } from "./top-band-layout";

/** What the band tells an item when it draws it. */
export interface TopBandRenderContext {
    /** The band is drawing this copy only to measure it, out of sight: no focus, no live region, nothing that acts. */
    measuring: boolean;
}

export interface TopBandItem {
    /** Unique within the pane's band; registering the same id again replaces the item. */
    id: string;
    slot: BandSlot;
    /** Higher keeps its full form longer. */
    priority: number;
    full: (ctx: TopBandRenderContext) => JSX.Element;
    /** A smaller form (an icon or a chip, with the full reading as its tooltip). Without one the item can only be dropped. */
    compact?: (ctx: TopBandRenderContext) => JSX.Element;
}

/** Of what the band holds, what outranks what. Controls are the target, so they outrank everything. */
export const TOP_BAND_PRIORITY = { controls: 100, stats: 10 } as const;

const bands = new Map<string, ReturnType<typeof createSignal<readonly TopBandItem[]>>>();

function bandOf(blockId: string) {
    let band = bands.get(blockId);
    if (!band) bands.set(blockId, (band = createSignal<readonly TopBandItem[]>([])));
    return band;
}

/** The items of a pane's band, in registration order. */
export const topBandItems = (blockId: string): Accessor<readonly TopBandItem[]> => bandOf(blockId)[0];

/** Put `item` in the pane's band. Returns what takes it out again. */
export function registerTopBandItem(blockId: string, item: TopBandItem): () => void {
    const [, setItems] = bandOf(blockId);
    setItems((now) => {
        const at = now.findIndex((i) => i.id === item.id);
        return at < 0 ? [...now, item] : now.map((i, k) => (k === at ? item : i));
    });
    return () => {
        // Only this registration: a replacement under the same id stays.
        setItems((now) => (now.includes(item) ? now.filter((i) => i !== item) : now));
    };
}

/** The pane is gone: forget its band. */
export function releaseTopBand(blockId: string): void {
    bands.delete(blockId);
}
