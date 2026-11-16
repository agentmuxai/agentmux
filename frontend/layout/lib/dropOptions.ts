// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The drops a drag can make on a pane, as outcomes: for each one, the tree action a release there commits and
 * the layout that action leaves. The one place a (dragged thing, hovered pane, direction) becomes an action:
 * the shown drop (placementPreview.ts `showPlacement`) and the drop guides (dropGuides.ts) both take it from
 * here, so a guide can only offer what a release does, and shows it as the layout the release makes.
 *
 * A direction is not an outcome. What `Left` does depends on the tree: beside a pane in a row it adds a pane
 * to the row, beside one in a column it splits the pane; `OuterLeft` is the same drop as `Left` wherever the
 * pane's group does not run that way, or is the layout's root; and a drop next to the dragged pane's own place
 * moves nothing. So the outcomes are listed by running every direction through the reducer the drop uses
 * (`futureLayoutOf`) and keeping one of each layout that differs from the others and from the layout as it is.
 */

import { futureLayoutOf, type FutureLayout } from "./futureLayout";
import type { LayoutModel } from "./layoutModel";
import { computeMoveNode } from "./layoutTree";
import {
    DropDirection,
    LayoutTreeActionType,
    type LayoutNode,
    type LayoutTreeAction,
    type LayoutTreeComputeMoveNodeAction,
} from "./types";

/** What is dragged, and the pane it is over. */
export interface DropSubject {
    /** The pane the pointer is over. */
    targetId: string;
    /** The dragged node's id: a pane in this tree, or the stand-in id of a node that is not in it. */
    draggedId: string;
    /** Set when the dragged node is not in this tree (a pane from another tab, a tab pill's stand-in). */
    foreignNode?: LayoutNode;
    /** The dragged thing is a tab pill being placed as a pane (pillPlacement.ts). */
    pill?: boolean;
    /**
     * What the middle of the pane does: the two panes swap (a pane of this tree), the tab joins the pane's tabs
     * (a tab pill), or nothing (a pane from another tab; a tab over its own pane). Default: by the two above.
     */
    center?: "swap" | "join" | "none";
}

/** One drop on a pane: the direction that names it, and the layout a release there leaves. */
export interface DropOption {
    direction: DropDirection;
    /** The tab joins the hovered pane's tabs: the layout stays as it is. */
    join?: true;
    /** The layout after the drop. None for a join. */
    future?: FutureLayout;
}

export type PlacementSide = "left" | "right" | "up" | "down";

/** The side of the hovered pane a direction puts the new pane on; undefined for the centre and for none. */
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

export function centerOf(subject: DropSubject): "swap" | "join" | "none" {
    return subject.center ?? (subject.pill ? "join" : subject.foreignNode ? "none" : "swap");
}

/** The move a drop of `subject` in `direction` asks the tree for. */
export function computeFor(subject: DropSubject, direction: DropDirection | undefined): LayoutTreeComputeMoveNodeAction {
    return {
        type: LayoutTreeActionType.ComputeMove,
        nodeId: subject.targetId,
        nodeToMoveId: subject.draggedId,
        direction,
        // The dragged node isn't in this tree: pass it so the drop can still be computed.
        nodeToMove: subject.foreignNode,
    } as LayoutTreeComputeMoveNodeAction;
}

/**
 * The drop of a node that is not in this tree: a tab pill that leaves a pane with other tabs, and a pane or a
 * tab that comes from another window tab. The hovered pane is split in half and the new pane takes the half on
 * the pointer's side, which is what the release does (`splitOutStackMember`, layoutStack.ts; srv's split for
 * a move between window tabs, crossTabDrag.ts). A split and not a move: a move puts the new pane beside the
 * hovered one at a size of its own, which is another layout than the one the release makes.
 */
export function splitFor(subject: DropSubject, direction: DropDirection | undefined): LayoutTreeAction | undefined {
    const side = sideOfDirection(direction);
    if (!side) return undefined;
    const horizontal = side === "left" || side === "right";
    return {
        type: horizontal ? LayoutTreeActionType.SplitHorizontal : LayoutTreeActionType.SplitVertical,
        targetNodeId: subject.targetId,
        newNode: subject.foreignNode,
        position: side === "left" || side === "up" ? "before" : "after",
        sizeFraction: 0.5,
    } as LayoutTreeAction;
}

/** Whether a drop of `subject` is the split of the hovered pane, not a move: every node that is not in this tree. */
export const dropsBySplit = (subject: DropSubject): boolean => !!subject.foreignNode;

/** The action a release of `subject` in `direction` commits, or none where the tree takes no drop. */
export function dropActionOf(model: LayoutModel, subject: DropSubject, direction: DropDirection | undefined): LayoutTreeAction | undefined {
    if (direction === undefined || subject.targetId === subject.draggedId) return undefined;
    if (dropsBySplit(subject)) return splitFor(subject, direction);
    if (direction === DropDirection.Center && centerOf(subject) !== "swap") return undefined;
    return computeMoveNode(model.treeState, computeFor(subject, direction)) ?? undefined;
}

/**
 * The layout as a drag is hit-tested against: its panes and their rects. They are the layout model's own rects
 * (`getNodeRectById`), which no drag changes (the tree is written only by the commit on drop, and a live reflow
 * moves panes with transforms the layout never reads). The one way they change under a drag is srv or another
 * window changing the tree.
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

/**
 * The pane a pointer at `offset` (the layout container's coordinates) is over by the layout's own rects, or
 * the nearest one from a gap between panes, with the pointer brought inside its rect. `skipId`: a pane that is
 * never the answer (the dragged one). The one reading of "which pane" for every drag that is not hit-tested
 * by the panes' own elements.
 */
export function nearestPane(
    model: LayoutModel,
    offset: { x: number; y: number },
    skipId?: string
): { nodeId: string; rect: Dimensions; offset: { x: number; y: number } } | undefined {
    let best: { nodeId: string; rect: Dimensions; dist: number } | undefined;
    for (const leaf of model.leafs()) {
        if (leaf.id === skipId) continue;
        const rect = model.getNodeRectById(leaf.id);
        if (!rect) continue;
        const dx = Math.max(rect.left - offset.x, 0, offset.x - (rect.left + rect.width));
        const dy = Math.max(rect.top - offset.y, 0, offset.y - (rect.top + rect.height));
        const dist = dx * dx + dy * dy;
        if (!best || dist < best.dist) best = { nodeId: leaf.id, rect, dist };
    }
    if (!best) return undefined;
    const r = best.rect;
    return {
        nodeId: best.nodeId,
        rect: r,
        offset: {
            x: Math.max(r.left, Math.min(r.left + r.width, offset.x)),
            y: Math.max(r.top, Math.min(r.top + r.height, offset.y)),
        },
    };
}

const px = (r: Dimensions): string => `${Math.round(r.left)},${Math.round(r.top)},${Math.round(r.width)},${Math.round(r.height)}`;

/** A layout as what the user sees of it: where each pane's content is, and where the dragged one is. */
function signatureOf(future: FutureLayout): string {
    return future.leafIds
        .map((id) => `${id === future.movedId ? "@moved" : (future.leafBlockIds[id] ?? id)}:${future.rects[id] ? px(future.rects[id]) : ""}`)
        .sort()
        .join(";");
}

function signatureNow(model: LayoutModel, draggedId: string): string {
    return model
        .leafs()
        .map((leaf) => {
            const r = model.getNodeRectById(leaf.id);
            return `${leaf.id === draggedId ? "@moved" : (leaf.data?.blockId ?? leaf.id)}:${r ? px(r) : ""}`;
        })
        .sort()
        .join(";");
}

/** Sides first, then the middle, then the drops beside the pane's whole group: the first of two alike is kept. */
const ORDER: DropDirection[] = [
    DropDirection.Top,
    DropDirection.Right,
    DropDirection.Bottom,
    DropDirection.Left,
    DropDirection.Center,
    DropDirection.OuterTop,
    DropDirection.OuterRight,
    DropDirection.OuterBottom,
    DropDirection.OuterLeft,
];

const isOuter = (d: DropDirection): boolean => d >= DropDirection.OuterTop && d <= DropDirection.OuterLeft;

function listOptions(model: LayoutModel, subject: DropSubject): DropOption[] {
    if (!model.treeState.rootNode) return [];
    // A node that is not in this tree lands beside the hovered pane, never across its whole group.
    const besideOnly = !!subject.foreignNode || !!subject.pill;
    const seen = new Set<string>([signatureNow(model, subject.draggedId)]);
    const options: DropOption[] = [];
    for (const direction of ORDER) {
        if (direction === DropDirection.Center) {
            if (centerOf(subject) === "join") options.push({ direction, join: true });
            if (centerOf(subject) !== "swap") continue;
        }
        if (besideOnly && isOuter(direction)) continue;
        const action = dropActionOf(model, subject, direction);
        const future = action && futureLayoutOf(model, action);
        if (!future) continue;
        const signature = signatureOf(future);
        if (seen.has(signature)) continue;
        seen.add(signature);
        options.push({ direction, future });
    }
    return options;
}

const cache = new Map<LayoutModel, { shape: string; byKey: Map<string, DropOption[]> }>();

/**
 * The drops `subject` can make on its hovered pane: one per layout a release can leave, none of them the
 * layout as it is. Listed once per pane for a drag: the same array comes back until the layout changes.
 */
export function dropOptionsOf(model: LayoutModel, subject: DropSubject): DropOption[] {
    const shape = layoutShape(model);
    let held = cache.get(model);
    if (!held || held.shape !== shape) {
        held = { shape, byKey: new Map() };
        cache.set(model, held);
    }
    const key = `${subject.targetId}|${subject.draggedId}|${subject.pill ? 1 : 0}|${subject.foreignNode?.id ?? ""}|${centerOf(subject)}`;
    let options = held.byKey.get(key);
    if (!options) {
        options = listOptions(model, subject);
        held.byKey.set(key, options);
    }
    return options;
}

/** The drag is over: the next one lists its drops again (a pane's tabs can change without its rect changing). */
export function resetDropOptions(): void {
    cache.clear();
}
