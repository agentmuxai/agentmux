// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Where a view stands inside its pane, for the window's trail of places (pane-history.ts): a view that wants
// its own place walked by Back and Forward registers a source for its pane while it is mounted. A view that
// has not is pane-level only. SPEC_BACK_FORWARD_FOLLOW_THE_WHOLE_TRAIL_2026_11_26.md §3.

import { untrack } from "solid-js";
import { getObjectValue, makeORef } from "@/app/store/mos";
import type { ViewPlace } from "./pane-history-core";

export interface PlaceSource {
    /** Where the view stands now. Cheap, and no layout read: it runs on every recorded step. */
    read(): ViewPlace | undefined;
    /** Put the view back at `place`, the way the view's own links reach it. False when that place is gone. */
    restore(place: ViewPlace): boolean | Promise<boolean>;
    /** False while the view has a request it has not taken yet (a link's one-shot ask), so its place is not final. */
    settled?(): boolean;
}

/** The views that register a source: a pane of one, just opened, is waited for before its place is read. */
export const PLACE_VIEWS: ReadonlySet<string> = new Set(["kanban"]);

/** A place larger than this is not a place (it holds content): the entry stays pane-level. */
const MAX_PLACE_CHARS = 1024;

const sources = new Map<string, PlaceSource>();

/** Register `source` for the pane `blockId`; the returned function unregisters it (only if it is still the one). */
export function registerPlaceSource(blockId: string, source: PlaceSource): () => void {
    sources.set(blockId, source);
    return () => {
        if (sources.get(blockId) === source) sources.delete(blockId);
    };
}

export function placeSourceOf(blockId: string): PlaceSource | undefined {
    return sources.get(blockId);
}

/** The place the view of `blockId` reports, or undefined (no source, nothing to say, or too large). Tracks nothing. */
export function readPlace(blockId: string): ViewPlace | undefined {
    const source = sources.get(blockId);
    if (!source) return undefined;
    try {
        const place = untrack(() => source.read());
        if (!place || Object.keys(place).length === 0) return undefined;
        return JSON.stringify(place).length <= MAX_PLACE_CHARS ? place : undefined;
    } catch {
        return undefined;
    }
}

/** Whether the pane's view is one that registers a source. */
export function joinsPlaces(blockId: string): boolean {
    const view = getObjectValue<Block>(makeORef("block", blockId))?.meta?.view;
    return typeof view === "string" && PLACE_VIEWS.has(view);
}

/**
 * The source of `blockId`, waiting up to `capMs` for a view that has just mounted to register it. Resolves at
 * once for a pane whose view does not join, and with undefined when none registers in time.
 */
export async function whenPlaceSource(blockId: string, capMs: number): Promise<PlaceSource | undefined> {
    const now = placeSourceOf(blockId);
    if (now || !joinsPlaces(blockId)) return now;
    const until = performance.now() + capMs;
    while (performance.now() < until) {
        await new Promise<void>((resolve) => (typeof requestAnimationFrame === "function" ? requestAnimationFrame(() => resolve()) : setTimeout(resolve, 16)));
        const found = placeSourceOf(blockId);
        if (found) return found;
    }
    return undefined;
}

/** Test hook. */
export function resetPlaceSourcesForTests(): void {
    sources.clear();
}
