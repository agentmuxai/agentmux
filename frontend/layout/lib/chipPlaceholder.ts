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
 * The row's chips as they stack once `dragged` is inserted at `index` (the index `moveNode` takes:
 * counted before the dragged pane leaves its old place): the stack it would join, over the expanded
 * pane it docks onto, and its place in it. Undefined when `parent` is not a Row or the chip would
 * not dock onto anything.
 */
function stackAfterDrop(parent: LayoutNode, dragged: LayoutNode, index: number) {
    if (parent.flexDirection !== FlexDirection.Row || !parent.children?.length) return undefined;
    const children = parent.children;
    const old = children.findIndex((c) => c.id === dragged.id);
    const rest = children.filter((c) => c.id !== dragged.id);
    const at = old !== -1 && old < index ? index - 1 : index;
    const after = [...rest.slice(0, at), dragged, ...rest.slice(at)];
    const targets = resolveRowSlipTargets(after);
    const dock = targets.get(dragged.id);
    if (!dock) return undefined;
    const others = after.filter((c) => c.id !== dragged.id && targets.get(c.id)?.id === dock.id);
    return { dock, others, place: after.slice(0, after.indexOf(dragged)).filter((c) => others.includes(c)).length };
}

/**
 * The drop placeholder for a dragged chip inserted at `index` in `parent`. Along the parent's
 * axis it is placed against `target` (the pane before the seam, or the first pane at index 0)
 * whose rect is `targetRect`. In a Row with an expanded sibling the chips stack vertically over it,
 * so the placeholder goes at the chip's place in that stack, wherever the pane before the seam is:
 * on the seam between its neighbours in the stack, or flush inside the stack's top or bottom edge.
 */
export function chipDropRect(
    parent: LayoutNode,
    dragged: LayoutNode,
    target: LayoutNode,
    targetIndex: number,
    index: number,
    rectOf: (node: LayoutNode) => Dimensions | undefined,
    chip: { width: number; height: number }
): Dimensions {
    const stack = stackAfterDrop(parent, dragged, index);
    if (stack) {
        const { others, place, dock } = stack;
        const first = rectOf(others[0] ?? dock);
        const last = rectOf(others[others.length - 1] ?? dock);
        const before = place > 0 ? rectOf(others[place - 1]) : undefined;
        if (first && last && (place === 0 || before)) {
            if (place === 0) return chipPlaceholderRect(first, chip, FlexDirection.Column, false, true);
            if (place === others.length) return chipPlaceholderRect(last, chip, FlexDirection.Column, true, true);
            return chipPlaceholderRect(before!, chip, FlexDirection.Column, true, false);
        }
    }
    const after = index > targetIndex;
    const outerEdge = !after || index >= (parent.children?.length ?? 1);
    return chipPlaceholderRect(
        rectOf(target) ?? { top: 0, left: 0, width: 0, height: 0 },
        chip,
        parent.flexDirection,
        after,
        outerEdge
    );
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
