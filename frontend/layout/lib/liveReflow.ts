// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live reflow of a pane drag. While a pane is dragged over the layout, every other pane's tile stands in
 * for the rect the drop would leave it (`futureLayoutOf`) with a compositor-only transform, a translate to its
 * future corner and a scale to its future size from its current rect (origin top-left), and the dragged pane's
 * own tile is hidden, so the gap is where it lands. Nothing here touches the tree, a tile's size or its layout
 * rect: the layout's own rects stay the drag's hit-test snapshot (placementPreview.ts), no ResizeObserver
 * fires, no terminal is resized, and the commit on drop stays the one tree write. A browser pane is a native
 * window a transform cannot move, so it is left where it is for now. Behind the `layout:livereflow` setting.
 * docs/specs/SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md §4, §5, §6.
 */

import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { getSettingsKeyAtom } from "@/app/store/block-atom-cache";
import { getObjectValue, makeORef } from "@/app/store/mos";
import type { LayoutModel } from "./layoutModel";
import type { LayoutNode } from "./types";

/** How a tile stands in for its future rect. */
export type ReflowEntry =
    /** The dragged pane: its tile is hidden, and the gap is where it lands. */
    | { kind: "hidden" }
    /** Another pane: its tile glides to its future rect under this transform. */
    | { kind: "glide"; transform: string };

/** With more panes than this the layout reflows without a glide: nothing moves until the drop (§5). */
export const REFLOW_MAX_PANES = 24;
/** A drag that sends no sample for this long has lost its end (a `dragend` that never came): the reflow lets go (§6). */
export const REFLOW_WATCHDOG_MS = 5000;
/** A rect within this of its future one has not moved. */
const STILL_PX = 0.5;

export interface LiveReflow {
    /** How the tile of `nodeId` stands in for its future rect now, or undefined when it stays as laid out. */
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
 * The transform that puts a tile laid out at `from` where `to` is: a translate to the future corner, as the
 * layout's own transform does (`setTransform`), and a scale to the future size, so the tile's width and height
 * never change and nothing inside it lays out.
 */
export function glideTransform(from: Dimensions, to: Dimensions): string {
    const translate = `translate3d(${Math.floor(to.left)}px,${Math.floor(to.top)}px, 0)`;
    const sx = from.width > 0 ? to.width / from.width : 1;
    const sy = from.height > 0 ? to.height / from.height : 1;
    if (Math.abs(sx - 1) < 1e-4 && Math.abs(sy - 1) < 1e-4) return translate;
    return `${translate} scale(${sx.toFixed(4)}, ${sy.toFixed(4)})`;
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

    const entries = createMemo((): Map<string, ReflowEntry> => {
        const none = new Map<string, ReflowEntry>();
        if (!enabled() || !model.activeDrag() || watchdogTripped()) return none;
        const future = model.futureLayout();
        if (!future) return none;
        const leafs = model.leafs();
        if (leafs.length > REFLOW_MAX_PANES) return none;
        const current = model.additionalProps();
        const futureRectOfBlock = new Map<string, Dimensions>();
        for (const id of future.leafIds) {
            const blockId = future.leafBlockIds[id];
            if (blockId && future.rects[id]) futureRectOfBlock.set(blockId, future.rects[id]);
        }
        const out = new Map<string, ReflowEntry>();
        for (const leaf of leafs) {
            if (isBrowserBlock(shownBlockId(leaf))) continue;
            if (leaf.id === future.movedId) {
                out.set(leaf.id, { kind: "hidden" });
                continue;
            }
            const from = current[leaf.id]?.rect;
            const to = leaf.data?.blockId ? futureRectOfBlock.get(leaf.data.blockId) : undefined;
            if (!from || !to || still(from, to)) continue;
            out.set(leaf.id, { kind: "glide", transform: glideTransform(from, to) });
        }
        return out;
    });

    return {
        reflowFor: (nodeId) => entries().get(nodeId),
        liveReflowActive: () => entries().size > 0,
    };
}
