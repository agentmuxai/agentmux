// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A single tab pill dragged over another pane (or its own, when it has two tabs or more): the
 * header and the middle of the body join the pill to that pane's tabs; the rest of the body is a
 * drop zone for a new pane next to it, shown with the standard placement ghost, exactly as when a
 * whole pane is dragged. The zone is a pure function of the pointer; the strip's drop target feeds
 * it and turns the result into feedback (`PaneTabStrip`), and the drop into `splitOutStackMember`.
 * docs/specs/SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md §3.1, §3.2.
 */

import { getNodeRectById } from "./layoutNodeModels";
import type { LayoutModel } from "./layoutModel";
import { placementDirection, showPlacement, sideOfDirection, type PlacementSide } from "./placementPreview";
import { DropDirection, LayoutTreeActionType, type LayoutNode } from "./types";

export type PillZone =
    /** Nothing here takes the pill (a pane's own header, or its own middle). */
    | { kind: "none" }
    /** The pill joins this pane's tabs. */
    | { kind: "join" }
    /** The pill becomes its own pane on `side` of this one, shown by the ghost for `direction`. */
    | { kind: "place"; direction: DropDirection; side: PlacementSide };

/** The stand-in for a pill in a preview: a leaf that is in no tree, as a pane from another tab is. */
export function pillStandIn(blockId: string): LayoutNode {
    return { id: `pill:${blockId}`, data: { blockId } } as LayoutNode;
}

export interface PillZoneInput {
    model: LayoutModel;
    /** The hovered pane. */
    nodeId: string;
    /** The dragged pill's block. */
    blockId: string;
    /** The pointer, in client coordinates. */
    pointer: { x: number; y: number };
    /** The hovered pane's header row, in client coordinates. */
    headerRect?: { left: number; top: number; right: number; bottom: number };
    /** The pill came from this pane: it can only be placed, never joined to where it already is. */
    ownPane: boolean;
    /** A header-only pane (minimized, collapsed) has no body to place against. */
    hasBody: boolean;
}

const inside = (p: { x: number; y: number }, r: NonNullable<PillZoneInput["headerRect"]>): boolean =>
    p.x >= r.left && p.x <= r.right && p.y >= r.top && p.y <= r.bottom;

/** What the pill would do if released at the pointer. */
export function pillZone(input: PillZoneInput): PillZone {
    const join: PillZone = input.ownPane ? { kind: "none" } : { kind: "join" };
    if (input.headerRect && inside(input.pointer, input.headerRect)) return join;
    if (!input.hasBody) return join;
    const container = input.model.displayContainerRef?.current;
    const rect = getNodeRectById(input.model, input.nodeId);
    if (!container || !rect) return join;
    const box = container.getBoundingClientRect();
    const direction = placementDirection(input.model, {
        targetId: input.nodeId,
        rect,
        offset: { x: input.pointer.x - box.x, y: input.pointer.y - box.y },
        draggedId: pillStandIn(input.blockId).id,
        foreignNode: pillStandIn(input.blockId),
    });
    const side = sideOfDirection(direction);
    if (direction === undefined || !side) return join;
    return { kind: "place", direction, side };
}

/** The same direction held again is not a new ghost: the caller keeps the last one it showed. */
export function showPillGhost(model: LayoutModel, input: PillZoneInput, zone: Extract<PillZone, { kind: "place" }>): void {
    const stand = pillStandIn(input.blockId);
    const container = model.displayContainerRef?.current;
    const rect = getNodeRectById(model, input.nodeId);
    if (!container || !rect) return;
    const box = container.getBoundingClientRect();
    showPlacement(
        model,
        {
            targetId: input.nodeId,
            rect,
            offset: { x: input.pointer.x - box.x, y: input.pointer.y - box.y },
            draggedId: stand.id,
            foreignNode: stand,
        },
        zone.direction
    );
}

/** Take the ghost down. */
export function clearPillGhost(model: LayoutModel): void {
    model.treeReducer({ type: LayoutTreeActionType.ClearPendingAction });
}
