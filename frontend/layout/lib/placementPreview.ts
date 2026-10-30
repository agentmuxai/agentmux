// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a dragged pane or pane tab would be placed next to the pane under the pointer, and the
 * ghost that shows it. The one place a pointer position becomes a `DropDirection` and a pending
 * `ComputeMove`: the tile overlay (`OverlayNode`, and its dead-spot fallback) and a tab pill
 * dragged over a pane's body (`paneBodyDrop`) both call it, so the ghost is the standard one.
 * docs/specs/SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md §3.2.
 */

import { chipOverChipDirection } from "./chipPlaceholder";
import { clampCrossTabDirection } from "./crossTabDrag";
import type { LayoutModel } from "./layoutModel";
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
    model.treeReducer(compute);
    return compute;
}

/** `placementDirection` and `showPlacement` in one step, for a drag that previews as it moves. */
export function previewPlacement(
    model: LayoutModel,
    input: PlacementInput
): { direction: DropDirection | undefined; compute: LayoutTreeComputeMoveNodeAction } {
    const direction = placementDirection(model, input);
    return { direction, compute: showPlacement(model, input, direction) };
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
