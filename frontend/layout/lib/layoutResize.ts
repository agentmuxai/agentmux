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
export const MinNodeSizePx = 40;

/**
 * Computes new sizes for every sibling in `siblings` when the one pane whose
 * edge is actually under the pointer (`drivenNodeId`) is resized to
 * `drivenDesiredSize`. The rest of `siblings` absorb the complementary delta
 * proportional to each one's *current* size (a pane holding more of the row
 * gives up more, one holding less gives up less) — not an equal split, and
 * not "snap everyone to the same value" (that's a different, selection-driven
 * model that doesn't fit a live drag; see SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md §3).
 *
 * Each sibling is floored at `minNodeSize`. If the combined floor headroom
 * across the other siblings is less than the drag calls for, `drivenNodeId`'s
 * own growth is capped to whatever was actually redistributable — this is
 * the same conservation-safety principle the plain 2-node resize already
 * applies to its one neighbor, generalized to N-1 neighbors instead of 1.
 * Shrinking `drivenNodeId` (giving size back to its siblings) has no such
 * cap: there's no max-size concept in this layout model, so growth is always
 * unconstrained. Pure function — no DOM/model dependency — so the
 * distribution math is directly unit-testable.
 *
 * SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md §5.2-§5.3.
 */
export function computeGroupResizeSizes(
    siblings: ResizeNodeOperation[],
    drivenNodeId: string,
    drivenDesiredSize: number,
    minNodeSize: number
): Map<string, number> {
    const result = new Map<string, number>(siblings.map((s) => [s.nodeId, s.size]));
    const driven = siblings.find((s) => s.nodeId === drivenNodeId);
    const others = siblings.filter((s) => s.nodeId !== drivenNodeId);
    if (!driven) return result;

    const clampedDesired = Math.max(drivenDesiredSize, minNodeSize);
    const totalDelta = clampedDesired - driven.size; // + = driven grows (others must shrink); - = driven shrinks (others grow)

    if (others.length === 0 || totalDelta === 0) {
        result.set(drivenNodeId, clampedDesired);
        return result;
    }

    if (totalDelta < 0) {
        // Driven shrinks; the space it gives up is unconstrained growth for
        // the others (no max-size floor to violate), so one proportional
        // pass is sufficient.
        const othersStartSum = others.reduce((sum, s) => sum + s.size, 0);
        const growTotal = -totalDelta;
        if (othersStartSum > 0) {
            for (const s of others) {
                result.set(s.nodeId, s.size + growTotal * (s.size / othersStartSum));
            }
        }
        result.set(drivenNodeId, clampedDesired);
        return result;
    }

    // Driven grows; the others must give up `totalDelta` combined, each
    // floored at `minNodeSize`. Iterative proportional shrink: a sibling
    // clamped to its floor drops out of the pool and its shortfall is
    // re-spread across whichever siblings are still above the floor. Each
    // round either fully satisfies the remaining delta (nobody clamps, the
    // while-loop exits on the next check) or permanently removes at least
    // one sibling from the pool — so this always terminates within
    // `others.length` rounds.
    let pool = others.map((s) => ({ id: s.nodeId, size: s.size }));
    let remaining = totalDelta;
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
    const actualDelta = totalDelta - Math.max(remaining, 0);
    result.set(drivenNodeId, driven.size + actualDelta);
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
