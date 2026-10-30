// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Minimizing a pane next to a minimized pane merges its tabs into that pane.
 * docs/specs/SPEC_MINIMIZE_MERGES_INTO_ADJACENT_MINIMIZED_PANE_2026_10_28.md.
 *
 * Only the person's own minimize merges: `mergeIntoMinimizedNeighbour`
 * (layoutMinimizeMergeApply.ts) runs from `minimizeNodeToggle` and `collapseMaximizedNode` (the
 * header button, shortcut, pane menu and palette), never from an agent's `Pane` minimize (srv), a
 * layout load or a restore. There is no undo; dragging a pill out of the merged pane is the way back.
 */

import { findNode, findParent } from "./layoutNode";
import { isEffectivelyMinimized, isNodeLocked } from "./layoutInvariants";
import { resolveRowSlipTargets } from "./layoutSlip";
import { FlexDirection, type LayoutNode } from "./types";

/**
 * The minimized pane `nodeId` would merge into: a sibling in the same split, the nearest minimized
 * leaf before it (above or left), else the nearest after, with no expanded pane in between. In a
 * Row a chip docks onto the nearest expanded sibling, so chips on either side of one expanded pane
 * stack together over it: those are adjacent too (`dockedTogether`). A fully-collapsed branch is a
 * group, not a pane: it is stepped over and never merged into. A pane in another split is never a
 * target. Pure.
 */
export function findMinimizeMergeTarget(root: LayoutNode | undefined, nodeId: string): LayoutNode | undefined {
    if (!root) return undefined;
    const node = findNode(root, nodeId);
    if (!node || node.children?.length || !node.data || isNodeLocked(node)) return undefined;
    const siblings = findParent(root, nodeId)?.children;
    if (!siblings) return undefined;
    const at = siblings.indexOf(node);
    for (const step of [-1, 1]) {
        for (let i = at + step; i >= 0 && i < siblings.length; i += step) {
            const sibling = siblings[i];
            if (!isEffectivelyMinimized(sibling)) break; // an expanded pane ends the run
            if (!sibling.children?.length && sibling.data) return sibling;
        }
    }
    return dockedTogether(findParent(root, nodeId), node);
}

/**
 * In a Row, a minimized sibling that docks onto the same expanded pane `node` would dock onto
 * once minimized: the two chips are stacked over that pane, so they are neighbours on screen
 * though an expanded pane sits between them in the tree. The nearest one before `node`, else after.
 */
function dockedTogether(parent: LayoutNode | undefined, node: LayoutNode): LayoutNode | undefined {
    const siblings = parent?.children;
    if (!siblings || parent.flexDirection !== FlexDirection.Row) return undefined;
    const dock = resolveRowSlipTargets(siblings, node.id).get(node.id);
    if (!dock) return undefined;
    const docksHere = resolveRowSlipTargets(siblings);
    const at = siblings.indexOf(node);
    const isChipHere = (c: LayoutNode) => !!c.minimized && !c.children?.length && !!c.data && docksHere.get(c.id)?.id === dock.id;
    for (let i = at - 1; i >= 0; i--) if (isChipHere(siblings[i])) return siblings[i];
    for (let i = at + 1; i < siblings.length; i++) if (isChipHere(siblings[i])) return siblings[i];
    return undefined;
}

/**
 * The expanded pane `nodeId`'s tabs go into when there is no minimized pane to merge into
 * (SPEC_MINIMIZE_MERGES_INTO_EXPANDED_NEIGHBOUR_2026_10_30.md §2.3): the same pane its chip would
 * dock onto, the nearest expanded sibling after it (right or below), else the nearest before it
 * (left or above). A sibling that is a split of several panes gives the leaf of it nearest to
 * `nodeId` (`nearestExpandedLeaf`). Only siblings of one split. Pure.
 */
export function findExpandedMergeTarget(root: LayoutNode | undefined, nodeId: string): LayoutNode | undefined {
    if (!root) return undefined;
    const node = findNode(root, nodeId);
    if (!node || node.children?.length || !node.data || isNodeLocked(node)) return undefined;
    const parent = findParent(root, nodeId);
    const siblings = parent?.children;
    if (!parent || !siblings) return undefined;
    const at = siblings.indexOf(node);
    for (const step of [1, -1]) {
        for (let i = at + step; i >= 0 && i < siblings.length; i += step) {
            const leaf = nearestExpandedLeaf(siblings[i], parent.flexDirection, step === 1);
            if (leaf) return leaf;
        }
    }
    return undefined;
}

/**
 * The expanded leaf of `node` nearest to a pane approaching from one side along `axis`: `node`
 * itself when it is a leaf; in a split along the same axis the first child (when `after`, the
 * approach is from before it) or the last; in a split across it every child touches the near edge,
 * so the first (leftmost, topmost). Minimized panes and fully-collapsed branches are stepped over.
 */
function nearestExpandedLeaf(node: LayoutNode, axis: FlexDirection, after: boolean): LayoutNode | undefined {
    if (!node.children?.length) return node.data && !isEffectivelyMinimized(node) ? node : undefined;
    const kids = node.flexDirection === axis && !after ? [...node.children].reverse() : node.children;
    for (const kid of kids) {
        const leaf = nearestExpandedLeaf(kid, axis, after);
        if (leaf) return leaf;
    }
    return undefined;
}
