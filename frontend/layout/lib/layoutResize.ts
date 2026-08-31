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
     *
     * (Since the mid-drag REBASE below, "drag start" here really means
     * "since the last modifier toggle" — a toggle rewrites these baselines
     * to the currently-staged sizes.)
     */
    groupSiblingStartSizes: ResizeNodeOperation[];
    /**
     * Which mode staged the previous tick. A tick arriving with the OTHER
     * mode is a mid-drag modifier toggle and triggers a rebase
     * (`rebaseResizeContextForModeSwitch`) instead of naively recomputing
     * the new mode's formula from the original drag-start baseline — which
     * would snap every border except the dragged one to where the new
     * formula would have put it had it run from the start (the "funny"
     * jump reported after the 2026-08-26 default flip).
     */
    lastGroupResize: boolean;
    /**
     * Every sibling's currently-staged size — seeded from the drag-start
     * sizes and merged with each successfully-staged tick's ops (a
     * floor-rejected direct-mode tick stages nothing and leaves this
     * untouched, matching the pending action). This is the rebase
     * baseline: what the user actually SEES at the moment of a toggle.
     */
    stagedSizes: Map<string, number>;
}

/**
 * Mid-drag modifier toggle: make the CURRENT visual state the new baseline
 * so the incoming mode's math applies only to post-toggle cursor motion.
 * Rewrites the 2-node start sizes and the group sibling snapshot to the
 * currently-staged sizes, and — the key move — `resizeHandleStartPx` to
 * the cursor's current position, so `clientDiff` restarts from zero.
 * Every border therefore stays exactly where it is on the toggle frame;
 * only subsequent movement follows the new mode. Toggling repeatedly
 * mid-drag just chains rebases. Exported for direct unit testing (pure
 * with respect to the context object — no DOM/model access).
 */
export function rebaseResizeContextForModeSwitch(
    ctx: ResizeContext,
    clientPoint: number,
    groupResize: boolean
): void {
    ctx.lastGroupResize = groupResize;
    ctx.resizeHandleStartPx = clientPoint;
    ctx.beforeNodeStartSize = ctx.stagedSizes.get(ctx.beforeNodeId) ?? ctx.beforeNodeStartSize;
    ctx.afterNodeStartSize = ctx.stagedSizes.get(ctx.afterNodeId) ?? ctx.afterNodeStartSize;
    ctx.groupSiblingStartSizes = ctx.groupSiblingStartSizes.map((s) => ({
        nodeId: s.nodeId,
        size: ctx.stagedSizes.get(s.nodeId) ?? s.size,
    }));
}

export const DefaultGapSizePx = 3;
// 128px minimum in both directions — this same constant floors both Row (width)
// and Column (height) drags, since minNodeSize is derived generically from
// whichever parent's pixelToSizeRatio is active (see onResizeMove below).
// Exported for the Shift+window-edge resize path (windowEdgeResize.ts), which
// applies the same floor directly in CSS px.
export const MinNodeSizePx = 128;

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
    // A member already at/under the floor has zero shrink headroom to give up.
    // Exclude it from the pool up front rather than discovering that inside the
    // loop: `cur - minNodeSize` for such a member is <= 0, which would otherwise
    // ENLARGE it up to minNodeSize (the opposite of "shrinking" this block) and
    // feed a non-positive amount into `takenThisRound`, breaking conservation
    // (the block could net *grow* instead of shrink, with the complementary
    // block never told to compensate). Already-undersized panes are a real,
    // reachable state here — split, minimize-restore, and window-shrink reflow
    // don't enforce minNodeSize, only this interactive drag path does (see
    // SPEC_SHIFT_DRAG_GROUP_RESIZE_DIRECTION_FIX_2026_08_17.md). Excluded
    // members are simply left at their current (possibly already-undersized)
    // size, never touched.
    let pool = block.filter((s) => s.size > minNodeSize).map((s) => ({ id: s.nodeId, size: s.size }));
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

    const beforeBlock = siblings.slice(0, drivenIndex);
    const afterBlock = siblings.slice(drivenIndex); // includes driven itself

    if (beforeBlock.length === 0) {
        // No block to redistribute with — driven alone is floored directly (degenerate
        // fallback; cannot happen via onResizeMove in production, see the doc comment above).
        result.set(drivenNodeId, Math.max(drivenDesiredSize, minNodeSize));
        return result;
    }

    // Deliberately UNCLAMPED: floor enforcement happens per-member inside shrinkBlockBy,
    // scoped to whichever block actually shrinks. Pre-clamping drivenDesiredSize to
    // minNodeSize here (as the prior single-node model correctly did, since driven's
    // final size WAS this value directly) would freeze totalDelta the instant the raw
    // cursor position implies driven should go below the floor — even though driven's
    // REAL final size, once shared proportionally across afterBlock, generally lands
    // well above the floor when afterBlock has other members. That stops the drag from
    // ever reaching the block's true (larger) headroom: the pane you're watching stalls
    // above minNodeSize and further dragging does nothing. See
    // SPEC_SHIFT_DRAG_GROUP_RESIZE_DIRECTION_FIX_2026_08_17.md §5 for the worked example.
    const totalDelta = drivenDesiredSize - driven.size; // + = afterBlock grows / beforeBlock shrinks; - = afterBlock shrinks / beforeBlock grows
    if (totalDelta === 0) {
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
        // NOT `parentIndex + 1`: a handle can legitimately span a zero-extent
        // slip child, so its two sides need not be adjacent in the child array
        // (see ResizeHandleProps.afterIndex). `??` guards a stale handle
        // produced by a frame from before this field existed.
        const afterNode = parentNode.children![resizeHandle.afterIndex ?? resizeHandle.parentIndex + 1];

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
                lastGroupResize: groupResize,
                stagedSizes: new Map(groupSiblingStartSizes.map((s) => [s.nodeId, s.size])),
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
    // Mid-drag modifier toggle → rebase BEFORE computing clientDiff, so
    // the new mode measures motion from the toggle point, not drag start.
    if (groupResize !== model.resizeContext.lastGroupResize) {
        rebaseResizeContextForModeSwitch(model.resizeContext, clientPoint, groupResize);
    }
    const clientDiff = (model.resizeContext.resizeHandleStartPx - clientPoint) * model.resizeContext.pixelToSizeRatio;
    const minNodeSize = MinNodeSizePx * model.resizeContext.pixelToSizeRatio;
    const afterNodeSize = model.resizeContext.afterNodeStartSize + clientDiff;

    let resizeOperations: ResizeNodeOperation[];
    if (groupResize) {
        // Default (no modifier) as of the 2026-08-26 flip: the pane whose edge
        // is under the pointer (afterNode, by the existing convention above)
        // drives the drag; every other sibling under the same parent absorbs
        // the complementary delta proportionally, instead of only the one
        // immediate neighbor. Shift+drag selects the direct 2-node transfer
        // in the else-branch below.
        // SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md §5.2 (the math);
        // SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26.md §2 (the flip).
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

    // Record what this tick staged — the rebase baseline for a future
    // mid-drag modifier toggle. Direct-mode floor rejections return above
    // without reaching here, correctly leaving the last staged state.
    for (const op of resizeOperations) {
        model.resizeContext.stagedSizes.set(op.nodeId, op.size);
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
