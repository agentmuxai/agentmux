// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { resolveRowSlipTargets } from "./layoutGeometry";
import { isNodeLocked } from "./layoutInvariants";
import { findNode, findParent } from "./layoutNode";
import { DropDirection, FlexDirection, type LayoutNode } from "./types";

/**
 * Where the drop placeholder goes when the pane being dragged is a minimized pane (a chip).
 *
 * It has the chip's own size (SPEC_DRAG_MINIMIZED_PANES_2026_10_20.md §3 A) and the expanded
 * placeholder's position: between two panes it is centred on the seam it will be inserted at,
 * half over each, and at the first or last place it is flush inside the edge
 * (SPEC_CHIP_DROP_PLACEHOLDER_ON_THE_SEAM_2026_10_21.md).
 *
 * `target` is the pane the drop is placed against, `after` whether it goes after that pane, and
 * `outerEdge` whether there is no pane on the other side of the seam.
 */
export function chipPlaceholderRect(
    target: Dimensions,
    chip: { width: number; height: number },
    direction: FlexDirection,
    after: boolean,
    outerEdge: boolean
): Dimensions {
    const along = (start: number, length: number, size: number) => {
        const seam = after ? start + length : start;
        if (!outerEdge) return seam - size / 2;
        return after ? seam - size : seam;
    };
    const height = Math.min(chip.height, target.height);
    if (direction === FlexDirection.Column) {
        return { left: target.left, width: target.width, height, top: along(target.top, target.height, height) };
    }
    const width = Math.min(chip.width, target.width);
    return { top: target.top, height, width, left: along(target.left, target.width, width) };
}

/**
 * The chips docked above the same expanded pane as `nodeId`, in row order, when `nodeId` is a
 * minimized child of a Row that has an expanded sibling: the geometry stacks them vertically over
 * that sibling (`resolveRowSlipTargets`), whatever the Row's own direction. Undefined for any other
 * pane, whose chips lie along the parent's axis.
 */
export function slipStack(parent: LayoutNode, nodeId: string): LayoutNode[] | undefined {
    if (parent.flexDirection !== FlexDirection.Row || !parent.children?.length) return undefined;
    const targets = resolveRowSlipTargets(parent.children);
    const dock = targets.get(nodeId);
    if (!dock) return undefined;
    return parent.children.filter((c) => targets.get(c.id)?.id === dock.id);
}

/**
 * The drop placeholder for a dragged chip inserted at `index` in `parent`, placed against `target`
 * (the pane before the seam, or the first pane at index 0) whose rect is `targetRect`. A chip in a
 * stack over an expanded pane is placed along the vertical axis, since that is where the seam shows.
 */
export function chipDropRect(
    parent: LayoutNode,
    target: LayoutNode,
    targetIndex: number,
    index: number,
    targetRect: Dimensions,
    chip: { width: number; height: number }
): Dimensions {
    const after = index > targetIndex;
    let outerEdge = !after || index >= (parent.children?.length ?? 1);
    let direction = parent.flexDirection;
    const stack = slipStack(parent, target.id);
    if (stack) {
        direction = FlexDirection.Column;
        outerEdge = after ? stack[stack.length - 1].id === target.id : stack[0].id === target.id;
    }
    return chipPlaceholderRect(targetRect, chip, direction, after, outerEdge);
}

/**
 * Where a chip dropped on another chip goes: to the nearer of the two seams around that chip, in
 * the direction the chips are laid out (the upper or lower half of a chip in a column or in the
 * stack over an expanded pane, the left or right half in a row of chips). A chip is thin, so the generic zones resolve most of its area to a swap
 * (centre) or a split of the chip (its ends), and the placeholder then covers the chip instead of
 * sitting between two (SPEC_CHIP_OVER_CHIP_DROP_2026_10_24.md).
 *
 * Undefined unless both nodes are chips of `root` with a parent to insert into, or when `offset`
 * is outside `rect`; the caller then uses the generic zones.
 */
export function chipOverChipDirection(
    root: LayoutNode | undefined,
    targetId: string,
    draggedId: string,
    rect: Dimensions | undefined,
    offset: { x: number; y: number } | null | undefined
): DropDirection | undefined {
    if (!root || !rect || !offset) return undefined;
    const target = findNode(root, targetId);
    const dragged = findNode(root, draggedId);
    const isChip = (n: LayoutNode | undefined) => !!n && !n.children?.length && isNodeLocked(n);
    if (!isChip(target) || !isChip(dragged)) return undefined;
    const parent = findParent(root, targetId);
    if (!parent) return undefined;
    const x = offset.x - rect.left;
    const y = offset.y - rect.top;
    if (x < 0 || y < 0 || x > rect.width || y > rect.height) return undefined;
    if (parent.flexDirection === FlexDirection.Column)
        return y < rect.height / 2 ? DropDirection.Top : DropDirection.Bottom;
    // The chips of a Row stack vertically over an expanded sibling: the upper half is before.
    if (slipStack(parent, targetId)) return y < rect.height / 2 ? DropDirection.Left : DropDirection.Right;
    return x < rect.width / 2 ? DropDirection.Left : DropDirection.Right;
}
