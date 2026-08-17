// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { debounce } from "throttle-debounce";
import { findNode } from "./layoutNode";
import { isEffectivelyMinimized } from "./layoutMinimize";
import {
    FlexDirection,
    LayoutTreeActionType,
    LayoutTreeResizeNodeAction,
    LayoutTreeSetPendingAction,
    ResizeHandleProps,
    ResizeNodeOperation,
} from "./types";
import type { LayoutModel } from "./layoutModel";
import { markEnd, markStart } from "@/perf";

export interface ResizeContext {
    handleId: string;
    pixelToSizeRatio: number;
    displayContainerRect?: Dimensions;
    resizeHandleStartPx: number;
    beforeNodeId: string;
    beforeNodeStartSize: number;
    afterNodeId: string;
    afterNodeStartSize: number;
    /**
     * Every non-minimize-locked sibling under the dragged handle's parent,
     * snapshotted at drag start (nodeId + starting size). Always populated —
     * not just when the group-resize modifier is initially held — so
     * toggling Shift mid-drag needs no context rebuild: each move tick just
     * picks which formula to apply to this same baseline + the current
     * drag-start-to-now pixel delta. SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md §5.4.
     */
    groupSiblingStartSizes: ResizeNodeOperation[];
}

export const DefaultGapSizePx = 3;
const MinNodeSizePx = 40;

/**
 * Shrinks `block` by `amount` in total, distributed proportionally to each
 * member's own current size (bigger members give up more), floored at
 * `minNodeSize`. A member clamped to its floor drops out of the pool and
 * its shortfall re-spreads across whichever members are still above the
 * floor — terminates within `block.length` rounds (each round either fully
 * satisfies `remaining` or permanently removes at least one member).
 * Mutates `result` in place; returns the amount actually shrunk (may be
 * less than `amount` if the block's combined floor headroom is smaller).
 */
function shrinkBlockBy(
    block: ResizeNodeOperation[],
    amount: number,
    minNodeSize: number,
    result: Map<string, number>
): number {
    let pool = block.map((s) => ({ id: s.nodeId, size: s.size }));
    let remaining = amount;
    while (remaining > 1e-6 && pool.length > 0) {
        const poolSum = pool.reduce((sum, p) => sum + result.get(p.id)!, 0);
        if (poolSum <= 0) break;
        const nextPool: typeof pool = [];
        let takenThisRound = 0;
        for (const p of pool) {
            const cur = result.get(p.id)!;
            const share = remaining * (cur / poolSum);
            const proposed = cur - share;
            if (proposed < minNodeSize) {
                takenThisRound += cur - minNodeSize;
                result.set(p.id, minNodeSize);
            } else {
                result.set(p.id, proposed);
                takenThisRound += share;
                nextPool.push(p);
            }
        }
        remaining -= takenThisRound;
        pool = nextPool;
    }
    return amount - Math.max(remaining, 0);
}

/**
 * Grows `block` by `amount` in total, distributed proportionally to each
 * member's own *starting* size (using `block`'s own sizes, not any prior
 * mutation of `result` in this call — matches how `shrinkBlockBy` computes
 * its pool). No floor/cap: there's no max-size concept in this layout
 * model. Mutates `result` in place.
 */
function growBlockBy(block: ResizeNodeOperation[], amount: number, result: Map<string, number>): void {
    if (amount <= 0 || block.length === 0) return;
    const blockSum = block.reduce((sum, s) => sum + s.size, 0);
    if (blockSum <= 0) {
        result.set(block[0].nodeId, block[0].size + amount);
        return;
    }
    for (const s of block) {
        result.set(s.nodeId, s.size + amount * (s.size / blockSum));
    }
}

/**
 * Computes new sizes for every sibling in `siblings` (in their parent's
 * child order) when the pane whose edge is actually under the pointer
 * (`drivenNodeId`) is dragged toward `drivenDesiredSize`.
 *
 * Modeled as two blocks meeting exactly at the dragged handle —
 * `beforeBlock` (every sibling ahead of `drivenNodeId`) and `afterBlock`
 * (`drivenNodeId` itself plus everyone after it) — rather than "one driven
 * node vs. an undifferentiated pool of others". The same aggregate transfer
 * amount `Δ` the raw pixel delta implies (`drivenDesiredSize - driven.size`)
 * is applied to the *blocks'* totals: `beforeBlock` changes by `-Δ`,
 * `afterBlock` by `+Δ`, exactly mirroring the plain 2-node baseline's
 * `beforeNode`/`afterNode` relationship, generalized from one node per side
 * to a block of them. Within each block, the change is distributed
 * proportionally to each member's own current size (uniform scaling) — not
 * an equal split, and not "snap everyone to the same value" (see
 * SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md §3).
 *
 * This guarantees the handle border (the boundary between the two blocks)
 * always tracks the pointer exactly, and — critically — that every OTHER
 * border in the group moves in the same direction as the drag, never
 * backward: for any block that scales uniformly, every border internal to
 * it shifts the same direction as the block's own boundary tied to the
 * drag. The prior "one driven node vs. every other sibling regardless of
 * position" model didn't have this property — a sibling positioned past
 * the driven node could get a growth share that pulled its shared border
 * *toward* the driven node, i.e. opposite the drag. See
 * SPEC_SHIFT_DRAG_GROUP_RESIZE_DIRECTION_FIX_2026_08_17.md for the full
 * worked example and the geometric argument.
 *
 * The trade-off: the driven pane's own size is no longer pixel-exact when
 * `afterBlock` has more than one member — it shares the block's change
 * with whatever sits past it. Only the handle border itself is guaranteed
 * to track the cursor 1:1; that's the invariant that actually matters for
 * a drag interaction.
 *
 * Each block is floored at `minNodeSize` when shrinking (unconstrained
 * when growing — no max-size concept exists). If the shrinking block's
 * combined floor headroom is less than `Δ` calls for, the growing block's
 * actual change is capped to whatever was actually redistributable — same
 * conservation-safety principle the plain 2-node resize already applies,
 * scoped to a block instead of a single neighbor. Pure function — no
 * DOM/model dependency — so the distribution math is directly
 * unit-testable.
 *
 * SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md §5.2-§5.3;
 * SPEC_SHIFT_DRAG_GROUP_RESIZE_DIRECTION_FIX_2026_08_17.md §4.
 */
export function computeGroupResizeSizes(
    siblings: ResizeNodeOperation[],
    drivenNodeId: string,
    drivenDesiredSize: number,
    minNodeSize: number
): Map<string, number> {
    const result = new Map<string, number>(siblings.map((s) => [s.nodeId, s.size]));
    const drivenIndex = siblings.findIndex((s) => s.nodeId === drivenNodeId);
    if (drivenIndex === -1) return result;
    const driven = siblings[drivenIndex];

    const clampedDesired = Math.max(drivenDesiredSize, minNodeSize);
    const totalDelta = clampedDesired - driven.size; // + = afterBlock grows / beforeBlock shrinks; - = afterBlock shrinks / beforeBlock grows

    const beforeBlock = siblings.slice(0, drivenIndex);
    const afterBlock = siblings.slice(drivenIndex); // includes driven itself

    if (beforeBlock.length === 0 || totalDelta === 0) {
        result.set(drivenNodeId, clampedDesired);
        return result;
    }

    if (totalDelta < 0) {
        // afterBlock (incl. driven) shrinks in total by -totalDelta; beforeBlock absorbs it as unconstrained growth.
        const actualDelta = shrinkBlockBy(afterBlock, -totalDelta, minNodeSize, result);
        growBlockBy(beforeBlock, actualDelta, result);
    } else {
        // beforeBlock shrinks in total by totalDelta, floored; afterBlock (incl. driven) grows by whatever was actually redistributable.
        const actualDelta = shrinkBlockBy(beforeBlock, totalDelta, minNodeSize, result);
        growBlockBy(afterBlock, actualDelta, result);
    }
    return result;
}

/**
 * Callback that is invoked when the TileLayout container is being resized.
 */
export function onContainerResize(model: LayoutModel) {
    model.updateTree();
    model.setter(model.isContainerResizing, true);
    model.stopContainerResizing();
}

/**
 * Create a debounced function to restore animations once the TileLayout container is no longer being resized.
 */
export function createStopContainerResizing(model: LayoutModel) {
    return debounce(30, () => {
        model.setter(model.isContainerResizing, false);
    });
}

/**
 * Callback to update pending node sizes when a resize handle is dragged.
 * @param model The LayoutModel instance.
 * @param resizeHandle The resize handle that is being dragged.
 * @param x The X coordinate of the pointer device, in CSS pixels.
 * @param y The Y coordinate of the pointer device, in CSS pixels.
 */
export function onResizeMove(
    model: LayoutModel,
    resizeHandle: ResizeHandleProps,
    x: number,
    y: number,
    groupResize: boolean = false
) {
    // Phase 0 perf instrumentation A1: time the splitter-drag hot
    // path. Each call is one mousemove → tree-size-recompute pass;
    // hypothesis H1 in the perf spec is that this is where pane
    // resize feels slow because every iteration fires a
    // browser_pane_resize IPC per pane via the model's effect chain.
    // The Long Tasks observer + IPC roundtrip clock cover the
    // downstream cost; this mark covers the synchronous compute.
    markStart("pane-resize-tick");
    const parentIsRow = resizeHandle.flexDirection === FlexDirection.Row;

    // If the resize context is out of date, update it and save it for future events.
    if (model.resizeContext?.handleId !== resizeHandle.id) {
        const parentNode = findNode(model.treeState.rootNode, resizeHandle.parentNodeId);
        const beforeNode = parentNode.children![resizeHandle.parentIndex];
        const afterNode = parentNode.children![resizeHandle.parentIndex + 1];

        // Minimized is a locked state: locked edges get no handle (layoutGeometry),
        // but a stale handle from a pre-suppression frame could still deliver a drag
        // here — refuse to build a resize context that flanks a locked node.
        if (isEffectivelyMinimized(beforeNode) || isEffectivelyMinimized(afterNode)) return;

        const addlProps = model.getter(model.additionalProps);
        const pixelToSizeRatio = addlProps[resizeHandle.parentNodeId]?.pixelToSizeRatio;
        if (beforeNode && afterNode && pixelToSizeRatio) {
            const groupSiblingStartSizes: ResizeNodeOperation[] = (parentNode.children ?? [])
                .filter((child) => !isEffectivelyMinimized(child))
                .map((child) => ({ nodeId: child.id, size: child.size }));
            model.resizeContext = {
                handleId: resizeHandle.id,
                displayContainerRect: model.displayContainerRef.current?.getBoundingClientRect(),
                resizeHandleStartPx: resizeHandle.centerPx,
                beforeNodeId: beforeNode.id,
                afterNodeId: afterNode.id,
                beforeNodeStartSize: beforeNode.size,
                afterNodeStartSize: afterNode.size,
                pixelToSizeRatio,
                groupSiblingStartSizes,
            };
        } else {
            console.error(
                "Invalid resize handle, cannot get the additional properties for the nodes in the resize handle properties."
            );
            return;
        }
    }

    const clientPoint = parentIsRow
        ? x - model.resizeContext.displayContainerRect?.left
        : y - model.resizeContext.displayContainerRect?.top;
    const clientDiff = (model.resizeContext.resizeHandleStartPx - clientPoint) * model.resizeContext.pixelToSizeRatio;
    const minNodeSize = MinNodeSizePx * model.resizeContext.pixelToSizeRatio;
    const afterNodeSize = model.resizeContext.afterNodeStartSize + clientDiff;

    let resizeOperations: ResizeNodeOperation[];
    if (groupResize) {
        // Shift held: the pane whose edge is under the pointer (afterNode, by
        // the existing convention above) drives the drag; every other
        // sibling under the same parent absorbs the complementary delta
        // proportionally, instead of only the one immediate neighbor.
        // SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md §5.2.
        const sizes = computeGroupResizeSizes(
            model.resizeContext.groupSiblingStartSizes,
            model.resizeContext.afterNodeId,
            afterNodeSize,
            minNodeSize
        );
        resizeOperations = Array.from(sizes.entries()).map(([nodeId, size]) => ({ nodeId, size }));
    } else {
        const beforeNodeSize = model.resizeContext.beforeNodeStartSize - clientDiff;
        if (beforeNodeSize < minNodeSize || afterNodeSize < minNodeSize) {
            return;
        }
        resizeOperations = [
            {
                nodeId: model.resizeContext.beforeNodeId,
                size: beforeNodeSize,
            },
            {
                nodeId: model.resizeContext.afterNodeId,
                size: afterNodeSize,
            },
        ];
    }

    const resizeAction: LayoutTreeResizeNodeAction = {
        type: LayoutTreeActionType.ResizeNode,
        resizeOperations,
    };
    const setPendingAction: LayoutTreeSetPendingAction = {
        type: LayoutTreeActionType.SetPendingAction,
        action: resizeAction,
    };

    model.treeReducer(setPendingAction);
    model.updateTree(false);
    markEnd("pane-resize-tick");
}

/**
 * Callback to end the current resize operation and commit its pending action.
 */
export function onResizeEnd(model: LayoutModel) {
    if (model.resizeContext) {
        model.resizeContext = undefined;
        model.treeReducer({ type: LayoutTreeActionType.CommitPendingAction });
    }
}
