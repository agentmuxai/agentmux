// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

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
 * Where a chip dropped on another chip goes: to the nearer of the two seams around that chip, in
 * the direction the chips are laid out (the upper or lower half of a chip in a column, the left or
 * right half in a row). A chip is thin, so the generic zones resolve most of its area to a swap
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
    return x < rect.width / 2 ? DropDirection.Left : DropDirection.Right;
}
