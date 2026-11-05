// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One block lives in one pane. A tree that holds a block in two places (two leaves, or a leaf and another leaf's
 * tab strip) draws it twice, and closing either closes both because they are one block. Nothing writes such a
 * tree on purpose; this module is how it is refused when a placement would make one, and healed when one is
 * found already: the first holder (depth first, in tree order) keeps the block, every later reference to it
 * goes. Pure over the tree, like `layoutInvariants.ts`, which reports the same condition as DUPLICATE_BLOCK.
 */

import type { LayoutNode, LayoutTreeState } from "./types";

/** Every block a leaf holds: its tab strip, else its one block. A branch holds none itself. */
export function blockIdsOfLeaf(node: LayoutNode): string[] {
    const stack = node.data?.blockStack;
    if (stack?.length) return stack;
    return node.data?.blockId ? [node.data.blockId] : [];
}

function walkLeaves(node: LayoutNode | undefined, visit: (leaf: LayoutNode) => void): void {
    if (!node) return;
    if (node.children?.length) node.children.forEach((c) => walkLeaves(c, visit));
    else if (node.data) visit(node);
}

/** The block ids held in more than one place in the tree, each once. */
export function duplicateBlockIds(root: LayoutNode | undefined): string[] {
    const seen = new Set<string>();
    const dupes = new Set<string>();
    walkLeaves(root, (leaf) => {
        // A block twice in one strip is a duplicate too.
        for (const id of blockIdsOfLeaf(leaf)) (seen.has(id) ? dupes : seen).add(id);
    });
    return [...dupes];
}

/** The first leaf holding `blockId`, if the tree has one. */
export function leafHolding(root: LayoutNode | undefined, blockId: string): LayoutNode | undefined {
    let found: LayoutNode | undefined;
    walkLeaves(root, (leaf) => {
        if (!found && blockIdsOfLeaf(leaf).includes(blockId)) found = leaf;
    });
    return found;
}

/**
 * Drops every later reference to a block the tree already holds, in place: from a leaf's tab strip (the leaf
 * stays while it has a member, its active tab moving to the first remaining one if the active one went), or the
 * whole leaf when nothing is left. A branch left with no children goes with its last leaf, and a branch left
 * with one child is the caller's balance pass to fold. Returns the block ids it removed a reference to.
 */
export function healDuplicateBlocks(root: LayoutNode | undefined): string[] {
    const removed: string[] = [];
    if (!root) return removed;
    const kept = new Set<string>();
    const heal = (node: LayoutNode): boolean => {
        if (node.children?.length) {
            node.children = node.children.filter(heal);
            return node.children.length > 0;
        }
        if (!node.data) return true;
        const members = blockIdsOfLeaf(node);
        const fresh = members.filter((id) => !kept.has(id));
        members.forEach((id) => kept.add(id));
        if (fresh.length === members.length) return true;
        removed.push(...members.filter((id) => !fresh.includes(id)));
        if (fresh.length === 0) return false;
        // The visible tab stays the one that was visible when it survives (`blockId` and `activeBlockId` name
        // it together everywhere else), else the first member left.
        const shown = [node.data.activeBlockId, node.data.blockId].find((id) => !!id && fresh.includes(id)) ?? fresh[0];
        node.data = { ...node.data, blockStack: fresh.length > 1 ? fresh : undefined, blockId: shown };
        if (fresh.length > 1) node.data.activeBlockId = shown;
        else delete node.data.activeBlockId;
        return true;
    };
    heal(root);
    return removed;
}

/** Whether `action` is a placement of a block the tree already holds (nothing to apply, nothing to send srv). */
export function placementRefused(state: LayoutTreeState, action: { type: string; node?: LayoutNode }): boolean {
    if (action.type !== "insert" && action.type !== "insertatindex") return false;
    return !!action.node && blockIdsOfLeaf(action.node).some((id) => !!leafHolding(state.rootNode, id));
}

/**
 * A placement of `node` is refused when the tree already holds the block it carries (opening an agent whose
 * pane is already in the layout must show that pane, never add a second one). Focus moves to the holder when
 * the placement asked for focus. True when the placement must not go ahead.
 */
export function alreadyHeld(state: LayoutTreeState, node: LayoutNode, focus: boolean | undefined): boolean {
    const holder = blockIdsOfLeaf(node)
        .map((id) => leafHolding(state.rootNode, id))
        .find((leaf) => !!leaf);
    if (!holder) return false;
    console.warn("[layout] a placement of a block the layout already holds was refused; showing its pane", holder.id);
    if (focus) state.focusedNodeId = holder.id;
    return true;
}
