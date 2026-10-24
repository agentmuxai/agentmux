// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure ordering helpers over the layout tree, shared by the multi-tab move and
 * its tests. No imports beyond node helpers, like `stackMembers.ts`.
 */

import type { LayoutNode } from "./types";
import { effectiveStack } from "./stackMembers";

/** Depth-first, children in order, before-children callback only; no imports so a test can mock the node helpers freely. */
function walk(node: LayoutNode | undefined, visit: (node: LayoutNode) => void): void {
    if (!node) return;
    visit(node);
    node.children?.forEach((child) => walk(child, visit));
}

/**
 * `wanted` block ids, in layout order: panes in tree order (depth first, children
 * in order), and within a pane in its tab-strip order. This is the order a
 * multi-tab move lands them in, so the result does not depend on the order the
 * user clicked. Ids that are in no pane are dropped; a repeated id appears once.
 * SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §4.3.
 */
export function blocksInLayoutOrder(root: LayoutNode | undefined, wanted: Iterable<string>): string[] {
    const want = new Set(wanted);
    const ordered: string[] = [];
    if (!root || want.size === 0) return ordered;
    walk(root, (node) => {
        if (!node.data) return;
        for (const id of effectiveStack(node.data)) {
            if (want.has(id) && !ordered.includes(id)) ordered.push(id);
        }
    });
    return ordered;
}

/** Every block id in the tree, for pruning a selection against the layout on screen. */
export function allBlockIds(root: LayoutNode | undefined): Set<string> {
    const ids = new Set<string>();
    if (!root) return ids;
    walk(root, (node) => {
        if (node.data) for (const id of effectiveStack(node.data)) ids.add(id);
    });
    return ids;
}
