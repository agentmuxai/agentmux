// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live reflow of a pane drag. While a pane is dragged over the layout, every pane's tile, the dragged one
 * included, takes the rect the armed drop would leave it (`futureLayoutOf`): its real position and its real
 * size, so its content lays out as it will after the drop and the layout is covered edge to edge, with nothing
 * cropped, stretched or left empty. The drop then commits what is already on screen.
 *
 * A tile's place glides (a transform), and its size is set at once but shown through a clip that glides from
 * the size it had to the size it gets, so the edges of neighbours move together and no gap or overlap opens
 * while they travel. A tile is laid out once per armed candidate: one that grows on an axis takes the new size
 * at once (the clip uncovers it), and one that shrinks keeps the larger size under the closing clip until the
 * glide is over (`REFLOW_SETTLE_MS`), then takes its real size. So a candidate that is replaced inside the
 * glide never costs the shrinking panes a layout.
 *
 * Nothing here touches the tree or the layout's own rects: they stay the drag's hit-test snapshot
 * (placementPreview.ts), and the commit on drop stays the one tree write. Terminals hold their grid until the
 * drag ends (pane-size-hold.ts). A browser pane is a native window that neither a transform nor a clip
 * moves, so it is left where it is for now. Behind the `layout:livereflow` setting.
 * docs/specs/SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md §4, §5, §6;
 * docs/reports/REPORT_LIVE_REFLOW_REAL_SIZES_2026_11_10.md.
 */

import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { paneCaptureInFlight } from "@/app/drag/pane-capture-gate";
import { DURATION_BASE_MS } from "@/app/motion/tokens";
import { holdPaneSizes } from "@/app/platform/pane-size-hold";
import { getSettingsKeyAtom } from "@/app/store/block-atom-cache";
import { getObjectValue, makeORef } from "@/app/store/mos";
import type { LayoutModel } from "./layoutModel";
import type { LayoutNode } from "./types";

/** A size in whole CSS px. */
export interface BoxSize {
    width: number;
    height: number;
}

/** How a tile takes part in the reflow while a drag is live. */
export interface ReflowEntry {
    /**
     * The rect the tile shows, in whole px as the layout's own transform rounds them (`setTransform`): where
     * the armed drop leaves the pane, or its own layout rect while no drop is armed.
     */
    rect: Dimensions;
    /**
     * The size the tile is laid out at: `rect`'s, or larger on an axis while the glide to a smaller rect is
     * running (the clip shows `rect`'s size either way).
     */
    box: BoxSize;
    /** The dragged pane, in its landing rect: the drop's own preview, so not blurred as a dragged tile otherwise is. */
    moved: boolean;
}

/** With more panes than this the layout does not reflow: nothing moves until the drop (§5). */
export const REFLOW_MAX_PANES = 24;
/** How long after a candidate arms a shrinking tile takes its real size: the glide's length, and a frame or two. */
export const REFLOW_SETTLE_MS = DURATION_BASE_MS + 40;

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

/** `rect` in the whole px the layout's own transform uses: the corner rounded down, the size up (`setTransform`). */
export function wholePx(rect: Dimensions): Dimensions {
    return {
        left: Math.floor(rect.left),
        top: Math.floor(rect.top),
        width: Math.ceil(rect.width),
        height: Math.ceil(rect.height),
    };
}

/**
 * The crop that shows `size` of a tile from its top-left corner, whatever size the tile is laid out at: the
 * insets are measured from the far edges (`100% - px`), so a transition between two of these moves the shown
 * edge in a straight line while the tile's own size steps underneath it.
 */
export function clipTo(size: BoxSize): string {
    return `inset(0px calc(100% - ${size.width}px) calc(100% - ${size.height}px) 0px)`;
}

/**
 * The size a tile is laid out at for a new target: the larger, on each axis, of the size it is laid out at now
 * and the size it gets. Growth is laid out at once, to be uncovered; a shrink waits under the clip.
 */
export function envelope(laidOut: BoxSize, target: BoxSize): BoxSize {
    return { width: Math.max(laidOut.width, target.width), height: Math.max(laidOut.height, target.height) };
}

/** The landing rect of the dragged pane: where the armed drop leaves it, or none while no drop is armed. */
export function landingRectOf(model: Pick<LayoutModel, "futureLayout">): Dimensions | undefined {
    const future = model.futureLayout();
    return future?.rects[future.movedId];
}

interface Targets {
    byId: Map<string, { rect: Dimensions; moved: boolean }>;
    /** Whether a drop is armed: the tiles show it, not their own layout. */
    showing: boolean;
}

/** The reflow entries for a drag of `model`, kept in the model's reactive owner. */
export function createLiveReflow(model: LayoutModel): LiveReflow {
    const enabled = getSettingsKeyAtom("layout:livereflow");

    // Where each tile goes. While a drag is live every pane the reflow can move has a target, even when no
    // candidate is armed (then it is its own layout rect), so a tile's transition stays on from the drag's
    // start to its end and only the end takes it off, in the same batch as the drop's real rects. Nothing
    // moves while the dragged pane's picture is still being taken: it is a capture of the window, and must
    // show the pane as it stood (pane-capture-gate.ts).
    const targets = createMemo((): Targets | undefined => {
        if (!enabled() || !model.activeDrag() || paneCaptureInFlight()) return undefined;
        const leafs = model.leafs();
        if (leafs.length > REFLOW_MAX_PANES) return undefined;
        const future = model.futureLayout();
        const current = model.additionalProps();
        const futureRectOfBlock = new Map<string, Dimensions>();
        for (const id of future?.leafIds ?? []) {
            const blockId = future!.leafBlockIds[id];
            if (blockId && future!.rects[id]) futureRectOfBlock.set(blockId, future!.rects[id]);
        }
        const byId = new Map<string, { rect: Dimensions; moved: boolean }>();
        for (const leaf of leafs) {
            if (isBrowserBlock(shownBlockId(leaf))) continue;
            const from = current[leaf.id]?.rect;
            if (!from) continue;
            // The dragged pane keeps its node id through the preview: its future rect is the landing rect.
            const moved = !!future && leaf.id === future.movedId;
            const to = moved ? future!.rects[leaf.id] : leaf.data?.blockId ? futureRectOfBlock.get(leaf.data.blockId) : undefined;
            byId.set(leaf.id, { rect: wholePx(to ?? from), moved: moved && !!to });
        }
        return { byId, showing: !!future && byId.size > 0 };
    });

    // Terminals keep their grid for as long as tiles can take a size the drop may never commit.
    createEffect(() => {
        if (!targets()) return;
        holdPaneSizes(model, true);
        onCleanup(() => holdPaneSizes(model, false));
    });

    // The targets whose glide is over: every tile then has exactly its rect's size.
    const [settled, setSettled] = createSignal<Targets | undefined>(undefined);
    createEffect(
        on(targets, (t) => {
            if (!t) return;
            const timer = setTimeout(() => setSettled(t), REFLOW_SETTLE_MS);
            onCleanup(() => clearTimeout(timer));
        })
    );

    const entries = createMemo<Map<string, ReflowEntry>>((before) => {
        const t = targets();
        const out = new Map<string, ReflowEntry>();
        if (!t) return out;
        const done = settled() === t;
        const current = model.additionalProps();
        for (const [id, target] of t.byId) {
            const size = { width: target.rect.width, height: target.rect.height };
            // What the tile is laid out at now: its box from the previous target, or its own layout size.
            const laidOut = before.get(id)?.box ?? wholePx(current[id].rect);
            out.set(id, { rect: target.rect, box: done ? size : envelope(laidOut, size), moved: target.moved });
        }
        return out;
    }, new Map());

    return {
        reflowFor: (nodeId) => entries().get(nodeId),
        liveReflowActive: () => !!targets()?.showing,
    };
}
