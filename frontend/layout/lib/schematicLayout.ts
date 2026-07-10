// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { FlexDirection, LayoutNode } from "./types";

/**
 * Recursively subdivides `boundingRect` among `rootNode`'s tree by each
 * node's `size` share along its `flexDirection` — the same flex-pool math
 * as layoutGeometry.ts's `updateTreeHelper` (pixelToSizeRatio = totalSize /
 * nodePixels), but standalone and side-effect-free: it does not read or
 * write any `LayoutModel` signal, so it's safe to call against a tab that
 * isn't the active one (its real `LayoutModel` still exists and is live —
 * see layoutModelHooks.ts — but its DOM is `display:none`, so the real
 * `updateTree`/`updateTreeHelper` path can't be reused for a hover preview
 * without corrupting that tab's actual render state).
 *
 * Returns a flat map of every node id (branches and leaves) to its rect
 * within `boundingRect`'s coordinate space.
 */
export function computeSchematicRects(rootNode: LayoutNode, boundingRect: Dimensions): Map<string, Dimensions> {
    const rects = new Map<string, Dimensions>();
    rects.set(rootNode.id, boundingRect);

    function recurse(node: LayoutNode, rect: Dimensions) {
        if (!node.children?.length) return;
        const isRow = node.flexDirection === FlexDirection.Row;
        const nodePixels = isRow ? rect.width : rect.height;
        if (nodePixels <= 0) return;
        const totalSize = node.children.reduce((acc, c) => acc + c.size, 0);
        if (totalSize <= 0) return;
        const ratio = totalSize / nodePixels;

        let cursor = isRow ? rect.left : rect.top;
        for (const child of node.children) {
            const childPixels = child.size / ratio;
            const childRect: Dimensions = isRow
                ? { top: rect.top, left: cursor, width: childPixels, height: rect.height }
                : { top: cursor, left: rect.left, width: rect.width, height: childPixels };
            rects.set(child.id, childRect);
            cursor += childPixels;
            recurse(child, childRect);
        }
    }

    recurse(rootNode, boundingRect);
    return rects;
}

/**
 * Walks a tree and returns every LEAF node (a real pane, not a branch)
 * paired with its schematic rect and blockId, skipping branches and any
 * leaf that didn't get a rect (e.g. a degenerate 0-size tree).
 */
export function schematicLeaves(
    rootNode: LayoutNode,
    rects: Map<string, Dimensions>
): { nodeId: string; blockId: string; rect: Dimensions }[] {
    const result: { nodeId: string; blockId: string; rect: Dimensions }[] = [];

    function walk(node: LayoutNode) {
        if (!node.children?.length) {
            const rect = rects.get(node.id);
            if (rect && node.data?.blockId) {
                result.push({ nodeId: node.id, blockId: node.data.blockId, rect });
            }
            return;
        }
        node.children.forEach(walk);
    }

    walk(rootNode);
    return result;
}
