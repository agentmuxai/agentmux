// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live reflow of a pane drag. While a pane is dragged over the layout, every other pane's tile stands in
 * for the rect the drop would leave it (`futureLayoutOf`) with a compositor-only change over its unchanged
 * size: a translate to its future corner, and a crop to its future size when that is smaller, so its content
 * keeps its real size and proportions (a scale squashed terminals). The dragged pane's own tile is hidden, so
 * the gap is where it lands. Nothing here touches the tree, a tile's size or its layout rect: the layout's own
 * rects stay the drag's hit-test snapshot (placementPreview.ts), no ResizeObserver fires, no terminal is
 * resized, and the commit on drop stays the one tree write. A browser pane is a native window a transform
 * cannot move, so it is left where it is for now. Behind the `layout:livereflow` setting.
 * docs/specs/SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md §4, §5, §6.
 */

import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { getSettingsKeyAtom } from "@/app/store/block-atom-cache";
import { getObjectValue, makeORef } from "@/app/store/mos";
import type { LayoutModel } from "./layoutModel";
import type { LayoutNode } from "./types";

/** How a tile takes part in the reflow while a drag is live. */
export type ReflowEntry =
    /** The dragged pane: its tile is hidden, and the gap is where it lands. */
    | { kind: "hidden" }
    /** Another pane whose rect the drop changes: its tile glides to its future rect, cropped to its future size. */
    | { kind: "glide"; transform: string; clip: string }
    /**
     * Another pane whose rect the drop leaves alone: its tile keeps the layout's own transform, but with the
     * glide's transition still on, so a pane that was mid-glide for the previous candidate glides back to its
     * rest instead of jumping there.
     */
    | { kind: "rest" };

/** The crop of a tile at rest: nothing cut, so a transition to a glide's crop interpolates. */
export const REST_CLIP = "inset(0px 0px 0px 0px)";
/** With more panes than this the layout reflows without a glide: nothing moves until the drop (§5). */
export const REFLOW_MAX_PANES = 24;
/** A drag that sends no sample for this long has lost its end (a `dragend` that never came): the reflow lets go (§6). */
export const REFLOW_WATCHDOG_MS = 5000;
/** A rect within this of its future one has not moved. */
const STILL_PX = 0.5;

export interface LiveReflow {
    /** How the tile of `nodeId` takes part in the reflow now, or undefined when the reflow is off. */
    reflowFor: (nodeId: string) => ReflowEntry | undefined;
    /** Whether the reflow is showing a drop now: the panes are the outline, so no wireframe and no dim. */
    liveReflowActive: () => boolean;
}

/** The block a pane shows: its active tab's, or its own. */
function shownBlockId(leaf: LayoutNode): string | undefined {
    return leaf.data?.activeBlockId ?? leaf.data?.blockId;
}

function isBrowserBlock(blockId: string | undefined): boolean {
    return !!blockId && getObjectValue<Block>(makeORef("block", blockId))?.meta?.view === "browser";
}

const still = (a: Dimensions, b: Dimensions): boolean =>
    Math.abs(a.left - b.left) < STILL_PX &&
    Math.abs(a.top - b.top) < STILL_PX &&
    Math.abs(a.width - b.width) < STILL_PX &&
    Math.abs(a.height - b.height) < STILL_PX;

/**
 * The glide of a tile laid out at `from` to `to`: a translate to the future corner, as the layout's own
 * transform does (`setTransform`), and a crop of the right and bottom edges to the future size where that is
 * smaller, so the tile's width and height never change, nothing inside it lays out, and its content keeps its
 * real size and proportions. A future rect that is larger shows the tile at its own size until the drop fills it.
 */
export function glideEntry(from: Dimensions, to: Dimensions): Extract<ReflowEntry, { kind: "glide" }> {
    const transform = `translate3d(${Math.floor(to.left)}px,${Math.floor(to.top)}px, 0)`;
    const right = Math.max(0, Math.round(from.width - to.width));
    const bottom = Math.max(0, Math.round(from.height - to.height));
    return { kind: "glide", transform, clip: `inset(0px ${right}px ${bottom}px 0px)` };
}

/** The reflow entries for a drag of `model`, kept in the model's reactive owner. */
export function createLiveReflow(model: LayoutModel): LiveReflow {
    const enabled = getSettingsKeyAtom("layout:livereflow");
    const [watchdogTripped, setWatchdogTripped] = createSignal(false);
    // Every new preview is a drag sample that restarts the watchdog; the drag's end clears it.
    createEffect(
        on([() => model.activeDrag(), () => model.futureLayout()], ([dragging]) => {
            setWatchdogTripped(false);
            if (!dragging) return;
            const timer = setTimeout(() => setWatchdogTripped(true), REFLOW_WATCHDOG_MS);
            onCleanup(() => clearTimeout(timer));
        })
    );

    // While a drag is live every pane the reflow can move has an entry, even when no candidate is armed (then
    // every one is at rest), so a tile's transition stays on from the drag's start to its end and only the end
    // takes it off, in the same batch as the drop's real rects.
    const state = createMemo((): { entries: Map<string, ReflowEntry>; showing: boolean } => {
        const none = { entries: new Map<string, ReflowEntry>(), showing: false };
        if (!enabled() || !model.activeDrag() || watchdogTripped()) return none;
        const leafs = model.leafs();
        if (leafs.length > REFLOW_MAX_PANES) return none;
        const future = model.futureLayout();
        const current = model.additionalProps();
        const futureRectOfBlock = new Map<string, Dimensions>();
        for (const id of future?.leafIds ?? []) {
            const blockId = future!.leafBlockIds[id];
            if (blockId && future!.rects[id]) futureRectOfBlock.set(blockId, future!.rects[id]);
        }
        const entries = new Map<string, ReflowEntry>();
        for (const leaf of leafs) {
            if (isBrowserBlock(shownBlockId(leaf))) continue;
            if (future && leaf.id === future.movedId) {
                entries.set(leaf.id, { kind: "hidden" });
                continue;
            }
            const from = current[leaf.id]?.rect;
            const to = leaf.data?.blockId ? futureRectOfBlock.get(leaf.data.blockId) : undefined;
            entries.set(leaf.id, !from || !to || still(from, to) ? { kind: "rest" } : glideEntry(from, to));
        }
        return { entries, showing: !!future && entries.size > 0 };
    });

    return {
        reflowFor: (nodeId) => state().entries.get(nodeId),
        liveReflowActive: () => state().showing,
    };
}
