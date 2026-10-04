// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Keyboard pane swap and resize (docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §9).

import { findNodeInDirection } from "./layoutFocus";
import type { LayoutModel } from "./layoutModel";
import { findNode, findParent } from "./layoutNode";
import {
    FlexDirection,
    LayoutTreeActionType,
    NavigateDirection,
    type LayoutNode,
    type LayoutTreeResizeNodeAction,
    type LayoutTreeSwapNodeAction,
    type ResizeNodeOperation,
} from "./types";

/** Share of the two panes' combined size one key press moves. */
export const KEYBOARD_RESIZE_STEP = 0.05;
/** Neither pane shrinks below this share of the two. */
const MIN_SHARE = 0.1;

/** Swaps the focused pane with the one next to it in `direction`. */
export function swapFocusedInDirection(model: LayoutModel, direction: NavigateDirection): boolean {
    const cur = model.focusedNodeId;
    if (!cur) return false;
    const { nodeId } = findNodeInDirection(model, cur, direction);
    if (!nodeId) return false;
    const action: LayoutTreeSwapNodeAction = { type: LayoutTreeActionType.Swap, node1Id: cur, node2Id: nodeId };
    model.treeReducer(action);
    return true;
}

/**
 * The size changes that move the focused pane's border one step in
 * `direction`, as Windows Terminal does: the nearest split along that axis
 * moves its border between the focused branch and its neighbour. Right/Down
 * grows the pane when it has a neighbour that way, otherwise shrinks it from
 * the other side. Null when there is no split on that axis; an empty list
 * when the border is already at its limit.
 */
export function keyboardResizeOperations(
    root: LayoutNode,
    focusedId: string,
    direction: NavigateDirection,
    step = KEYBOARD_RESIZE_STEP
): ResizeNodeOperation[] | null {
    const horizontal = direction === NavigateDirection.Left || direction === NavigateDirection.Right;
    const axis = horizontal ? FlexDirection.Row : FlexDirection.Column;
    const forward = direction === NavigateDirection.Right || direction === NavigateDirection.Down;
    let child = findNode(root, focusedId);
    let parent = child ? findParent(root, child.id) : undefined;
    while (child && parent) {
        const kids = parent.children ?? [];
        if (parent.flexDirection === axis && kids.length > 1) {
            const i = kids.findIndex((c) => c.id === child!.id);
            // `a` sits before `b`; moving their border forward grows `a`.
            const [a, b] = forward
                ? i + 1 < kids.length
                    ? [kids[i], kids[i + 1]]
                    : [kids[i - 1], kids[i]]
                : i > 0
                  ? [kids[i - 1], kids[i]]
                  : [kids[i], kids[i + 1]];
            const pair = a.size + b.size;
            const delta = pair * step * (forward ? 1 : -1);
            const sizeA = a.size + delta;
            const sizeB = b.size - delta;
            if (sizeA < pair * MIN_SHARE || sizeB < pair * MIN_SHARE) return [];
            return [
                { nodeId: a.id, size: sizeA },
                { nodeId: b.id, size: sizeB },
            ];
        }
        child = parent;
        parent = findParent(root, parent.id);
    }
    return null;
}

/** Moves the focused pane's border one step in `direction`. */
export function resizeFocusedInDirection(model: LayoutModel, direction: NavigateDirection): boolean {
    const root = model.treeState?.rootNode;
    const cur = model.focusedNodeId;
    if (!root || !cur) return false;
    const ops = keyboardResizeOperations(root, cur, direction);
    if (ops == null) return false;
    if (ops.length > 0) {
        const action: LayoutTreeResizeNodeAction = { type: LayoutTreeActionType.ResizeNode, resizeOperations: ops };
        model.treeReducer(action);
    }
    return true;
}
