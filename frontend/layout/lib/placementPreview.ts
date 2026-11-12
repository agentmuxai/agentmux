// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a dragged pane or pane tab would be placed next to the pane under the pointer, and the
 * ghost that shows it. The one place a pointer position becomes a `DropDirection` and a pending
 * `ComputeMove`: the tile overlay (`OverlayNode`, and its dead-spot fallback) and a tab pill
 * dragged over a pane's body (`paneBodyDrop`) both call it, so the ghost is the standard one.
 * docs/specs/SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md §3.2.
 */

import { isAnyUnderway, onSessionEnded } from "@/app/drag/drag-session";
import { chipOverChipDirection } from "./chipPlaceholder";
import { DEFAULT_TRACKER, DropIntentTracker } from "./dropIntentTracker";
import { dragState } from "./tilelayout-drag-state";
import { clampCrossTabDirection } from "./crossTabDrag";
import type { LayoutModel } from "./layoutModel";
import { leaveOwnPane, pointerOutsideWindow } from "./tearOffPreview";
import { DropDirection, LayoutTreeActionType, type LayoutNode, type LayoutTreeComputeMoveNodeAction } from "./types";
import { determineDropDirection } from "./utils";

export interface PlacementInput {
    /** The pane the pointer is over. */
    targetId: string;
    /** Its rectangle in the layout container's coordinates. */
    rect: Dimensions;
    /** The pointer, in the layout container's coordinates. */
    offset: { x: number; y: number };
    /** The dragged node's id: a pane in this tree, or the stand-in id of a node that is not in it. */
    draggedId: string;
    /**
     * Set when the dragged node is not in this tree (a pane from another tab, a tab pill): the ghost
     * is previewed from it, and `Outer*` is clamped to the inner direction, because the commit can only
     * split the hovered pane in half (`clampCrossTabDirection`).
     */
    foreignNode?: LayoutNode;
}

/** The direction under the pointer, or undefined where there is none (outside the pane, on a diagonal). */
export function placementDirection(model: LayoutModel, input: PlacementInput): DropDirection | undefined {
    const raw =
        (input.foreignNode
            ? undefined
            : chipOverChipDirection(
                  model.treeState.rootNode,
                  input.targetId,
                  input.draggedId,
                  input.rect,
                  input.offset
              )) ?? determineDropDirection(input.rect, input.offset);
    return input.foreignNode ? clampCrossTabDirection(raw) : raw;
}

/**
 * Show the ghost for `direction` over `input.targetId`: dispatches `ComputeMove`, and the layout's
 * placeholder draws the pending action. Returns the action it dispatched.
 */
export function showPlacement(
    model: LayoutModel,
    input: PlacementInput,
    direction: DropDirection | undefined
): LayoutTreeComputeMoveNodeAction {
    const compute = {
        type: LayoutTreeActionType.ComputeMove,
        nodeId: input.targetId,
        nodeToMoveId: input.draggedId,
        direction,
        // The dragged node isn't in this tree: pass it so the ghost can still be computed. Preview
        // only; the drop never commits this pending action locally.
        nodeToMove: input.foreignNode,
    } as LayoutTreeComputeMoveNodeAction;
    // A sample the throttle held back past the pointer's exit: the pointer is outside now, and what the
    // window shows is the tear-off (tearOffPreview.ts), which a placement would replace. Coming back is
    // the window's `dragenter`, not a stale sample.
    if (pointerOutsideWindow(model)) return compute;
    // The pointer is over another pane: no tear-off is shown.
    leaveOwnPane(model);
    // A drop is shown again: the zones the pointer left a moment ago clear nothing (`leavePlacement`).
    cancelLeave(model);
    model.treeReducer(compute);
    return compute;
}

/** What a model's drag is steadied by: its tracker, the latest input per hovered pane, and the dwell timer. */
interface Steadying {
    tracker: DropIntentTracker;
    cross: boolean;
    inputs: Map<string, PlacementInput>;
    timer?: ReturnType<typeof setTimeout>;
    /** The layout the armed candidate belongs to (`layoutShape`): another one means the tree changed under the drag. */
    shape?: string;
}

/**
 * The frozen hit-test snapshot: the panes and rects a drag is hit-tested against. They are the layout model's
 * own rects (`additionalProps().rect`, `getNodeRectById`), which no drag changes (the tree is written only by
 * the commit on drop, and a live reflow moves panes with transforms the layout never reads), so a candidate is
 * always chosen against the layout the drag started on. The one way they change under a drag is srv or another
 * window changing the tree; then the armed candidate belongs to a layout that is gone, and the tracker starts
 * again on the new one.
 */
export function layoutShape(model: LayoutModel): string {
    const leafs = model.leafs?.() ?? [];
    let shape = "";
    for (const leaf of leafs) {
        const r = model.getNodeRectById?.(leaf.id);
        shape += r ? `${leaf.id}:${r.left},${r.top},${r.width},${r.height};` : `${leaf.id};`;
    }
    return shape;
}

const steadying = new WeakMap<LayoutModel, Steadying>();
/** The models with a drag being steadied, so the end of a drag can reset the destination tab's too. */
const steadied = new Set<LayoutModel>();

function steadyingOf(model: LayoutModel, cross: boolean): Steadying {
    const have = steadying.get(model);
    if (have && have.cross === cross) return have;
    if (have) clearTimeout(have.timer);
    // A pane from another tab arms at once, however fast the pointer: its drop record is taken with every sample.
    const config = cross ? { ...DEFAULT_TRACKER, dwellMs: 0, fastPxPerMs: Infinity } : DEFAULT_TRACKER;
    const made = { tracker: new DropIntentTracker(config), cross, inputs: new Map<string, PlacementInput>() };
    steadying.set(model, made);
    steadied.add(model);
    return made;
}

/** The drag is over: the next one starts from nothing, and no waiting candidate shows a ghost after the drop. */
export function resetDropIntent(model: LayoutModel): void {
    const have = steadying.get(model);
    if (!have) return;
    clearTimeout(have.timer);
    steadying.delete(model);
    steadied.delete(model);
}

/**
 * Clear the ghost. A candidate still waiting must not bring it back, so its timer goes too; `reset` also drops
 * the armed candidate (the pointer is over the dragged pane itself, where there is none).
 */
export function clearPlacement(model: LayoutModel, reset = false): void {
    if (reset) resetDropIntent(model);
    else clearTimeout(steadying.get(model)?.timer);
    cancelLeave(model);
    model.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
}

/** The drags that show a drop in a layout. */
export const GHOST_DRAG_KINDS = ["tile", "window-tab", "pane-tab", "doc-tab"] as const;

/**
 * How long a drop stays shown after the pointer left the zones it was armed from, ms. Crossing from one pane's
 * zones to the next is a leave and then, a frame later, the next pane's first sample, which most often shows
 * the same drop again (the tracker holds it). Cleared at the leave, the drop was gone for that frame: the panes
 * started back to their own places, the pane in hand went back to its full size, the landing zone began its
 * exit with the layout dimmed around it, and all of it came forward again (the operator, 2026-11-12: "it snaps
 * in and snaps out really fast into a possible panel layout"; "a window-tab wide fade-in/out, very fast").
 */
export const LEAVE_GRACE_MS = 80;

/** The layouts whose drop a leave is about to clear, with the timer that does it. */
const leaves = new Map<LayoutModel, ReturnType<typeof setTimeout>>();

function cancelLeave(model: LayoutModel): void {
    const timer = leaves.get(model);
    if (timer === undefined) return;
    clearTimeout(timer);
    leaves.delete(model);
}

/**
 * The pointer left the zones a drop was shown from (a pane's overlay, or the layout itself). The drop is
 * cleared once `LEAVE_GRACE_MS` has passed with no sample showing one again: a pointer that only crossed to
 * the next pane's zones sends one within a frame, and one that has really left sends none. A candidate still
 * waiting must not bring the ghost back, so its timer goes at once. With no drag underway (the leave that a
 * cancelled drag ends with) there is nothing to wait for.
 */
export function leavePlacement(model: LayoutModel): void {
    if (!isAnyUnderway(GHOST_DRAG_KINDS)) {
        clearPlacement(model);
        return;
    }
    clearTimeout(steadying.get(model)?.timer);
    cancelLeave(model);
    leaves.set(
        model,
        setTimeout(() => {
            leaves.delete(model);
            model.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
        }, LEAVE_GRACE_MS)
    );
}

// The drag is over. A drop has committed what was shown and cleared it; a cancelled drag left it to the grace,
// and nothing shows a drop of a drag that is over.
onSessionEnded(() => {
    for (const model of [...leaves.keys()]) clearPlacement(model);
});

/** The drag is over, in whichever tab it was steadied (a pane dragged to another tab has two). */
export function resetAllDropIntent(): void {
    for (const model of [...steadied]) resetDropIntent(model);
}

/**
 * `placementDirection` and `showPlacement` in one step, for a drag that previews as it moves. The ghost
 * follows the steadied candidate (dropIntentTracker.ts), which can be another pane's than the one the
 * pointer is over: `targetId` says which, and is what a caller records a drop against.
 */
export function previewPlacement(
    model: LayoutModel,
    input: PlacementInput
): { direction: DropDirection | undefined; compute: LayoutTreeComputeMoveNodeAction; targetId: string } {
    const raw = { targetId: input.targetId, direction: placementDirection(model, input) };
    const held = steadyingOf(model, !!input.foreignNode);
    clearTimeout(held.timer);
    const shape = layoutShape(model);
    if (held.shape !== undefined && held.shape !== shape) {
        // The tree changed under the drag: the armed candidate and the inputs describe panes that moved.
        held.tracker.reset();
        held.inputs.clear();
    }
    held.shape = shape;
    held.inputs.set(input.targetId, input);
    const now = performance.now();
    const { armed, flushAt } = held.tracker.step(input.draggedId, input.offset.x, input.offset.y, now, raw);
    if (flushAt !== undefined) {
        held.timer = setTimeout(() => showWaiting(model, held, input.draggedId), Math.max(0, flushAt - now));
    }
    const shown = held.inputs.get(armed.targetId) ?? input;
    return { direction: armed.direction, compute: showPlacement(model, shown, armed.direction), targetId: armed.targetId };
}

/** The dwell ran out with no new sample: show the candidate that was waiting. */
function showWaiting(model: LayoutModel, held: Steadying, draggedId: string): void {
    if (dragState.nodeId !== draggedId) return;
    const armed = held.tracker.armWaiting();
    const shown = armed && held.inputs.get(armed.targetId);
    if (armed && shown) showPlacement(model, shown, armed.direction);
}

/** The side of the hovered pane a direction puts the new pane on; undefined for the centre and for none. */
export type PlacementSide = "left" | "right" | "up" | "down";

export function sideOfDirection(direction: DropDirection | undefined): PlacementSide | undefined {
    switch (direction) {
        case DropDirection.Left:
        case DropDirection.OuterLeft:
            return "left";
        case DropDirection.Right:
        case DropDirection.OuterRight:
            return "right";
        case DropDirection.Top:
        case DropDirection.OuterTop:
            return "up";
        case DropDirection.Bottom:
        case DropDirection.OuterBottom:
            return "down";
        default:
            return undefined;
    }
}
