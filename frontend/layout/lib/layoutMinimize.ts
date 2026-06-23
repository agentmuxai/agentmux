// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


import { findNode, findParent } from "./layoutNode";
import type { LayoutModel } from "./layoutModel";

/** Height of the block header in CSS pixels (matches --header-height in theme.scss). */
export const HeaderHeightPx = 33;

/**
 * Toggle minimize for a given leaf node. Minimized panes collapse to their header
 * bar, redistributing freed space to an adjacent sibling. Restoring returns the
 * pane to its original size by reclaiming space from that same sibling.
 */
export function minimizeNodeToggle(model: LayoutModel, nodeId: string) {
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node) return;

    const parentNode = findParent(model.treeState.rootNode, nodeId);
    if (!parentNode?.children?.length) return;

    const addlProps = model.getter(model.additionalProps);
    const pixelToSizeRatio = addlProps[parentNode.id]?.pixelToSizeRatio;
    if (!pixelToSizeRatio) return;

    const siblings = parentNode.children;
    const nodeIdx = siblings.findIndex((c) => c.id === nodeId);

    // Pick the adjacent sibling that will absorb or yield space.
    const sibling = siblings[nodeIdx + 1] ?? siblings[nodeIdx - 1];
    if (!sibling) return;

    // Add gap so the header isn't clipped by the tile-leaf's gap/2 inset padding on each side.
    const gapSizePx = model.gapSizePx();
    const headerSizeUnits = (HeaderHeightPx + gapSizePx) * pixelToSizeRatio;

    if (node.minimizedSize === undefined) {
        // --- Minimize ---
        const freedUnits = node.size - headerSizeUnits;
        if (freedUnits <= 0) return; // already tiny — no-op

        node.minimizedSize = node.size;
        node.size = headerSizeUnits;
        sibling.size += freedUnits;
    } else {
        // --- Restore ---
        const reclaimUnits = node.minimizedSize - headerSizeUnits;
        const siblingAfterReclaim = sibling.size - reclaimUnits;
        // Clamp: don't let sibling go below its own header height.
        const minSiblingUnits = HeaderHeightPx * pixelToSizeRatio;
        if (siblingAfterReclaim < minSiblingUnits) {
            const available = sibling.size - minSiblingUnits;
            node.size += available;
            sibling.size = minSiblingUnits;
        } else {
            node.size = node.minimizedSize;
            sibling.size = siblingAfterReclaim;
        }
        node.minimizedSize = undefined;
    }

    // Update the minimized-node-id set for reactive isMinimized checks.
    model.minimizedNodeIds._set((prev) => {
        const next = new Set(prev);
        if (node.minimizedSize !== undefined) {
            next.add(nodeId);
        } else {
            next.delete(nodeId);
        }
        return next;
    });

    model.updateTree();
    model.localTreeStateAtom._set({ ...model.treeState });
    model.persistToBackend();
}

/**
 * Scan the loaded tree for nodes that carry a `minimizedSize` field and rebuild
 * the in-memory `minimizedNodeIds` set. Called once during `initializeFromWaveObject`.
 */
export function rebuildMinimizedSet(model: LayoutModel) {
    const ids = new Set<string>();
    function walk(node: typeof model.treeState.rootNode) {
        if (!node) return;
        if (node.minimizedSize !== undefined) ids.add(node.id);
        node.children?.forEach(walk);
    }
    walk(model.treeState.rootNode);
    model.minimizedNodeIds._set(ids);
}
