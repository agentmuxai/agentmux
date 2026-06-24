// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


import { newLayoutNode } from "./layoutNode";
import { findNode, findParent } from "./layoutNode";
import type { LayoutModel } from "./layoutModel";
import { DefaultNodeSize, FlexDirection, type LayoutNodeAdditionalProps, type LayoutNode } from "./types";

/** Height of the block header in CSS pixels (matches --header-height in theme.scss). */
export const HeaderHeightPx = 33;

/**
 * Toggle minimize for a given leaf node.
 *
 * There are two minimize paths depending on the node's parent flex direction:
 *
 * **Column parent (normal path):** The node collapses vertically to its header bar.
 * Freed space is given to the adjacent sibling. `node.minimizedSize` stores the
 * original height for restore.
 *
 * **Row parent (slip path):** Shrinking the node's Row-slot size would make it
 * horizontally narrow (thin strip) rather than vertically collapsed. Instead the
 * pane "slips" its header to the top of the nearest adjacent column, giving its
 * full width to that column. `node.slipMinimize` stores restore context.
 *
 * ### balanceNode and the _slipAnchor invariant
 *
 * After a slip, the parentRow has exactly one child (the target Column). Normally
 * `balanceNode` would hoist that column's children into the grandparent (single-
 * child-branch flatMap rule), scattering them and losing `targetColumnId`. We
 * prevent this by setting `parentRow._slipAnchor = true`, which `balanceNode`
 * checks before applying the hoist. The Row(→Column) direction alternation is
 * preserved, and the tree is stable through serialise/reload cycles.
 */
export function minimizeNodeToggle(model: LayoutModel, nodeId: string) {
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node) return;

    const addlProps = model.getter(model.additionalProps);
    const gapSizePx = model.gapSizePx();

    // ── Restore: slip path ────────────────────────────────────────────────────
    if (node.slipMinimize !== undefined) {
        const { targetColumnId, originalRowSize, originalRowIndex, targetWasLeaf } = node.slipMinimize;

        const targetCol = findNode(model.treeState.rootNode, targetColumnId);
        if (targetCol?.children) {
            // Remove the slipped header from the column; give its size back to the neighbor.
            const slipIdx = targetCol.children.findIndex((c) => c.id === nodeId);
            if (slipIdx !== -1) {
                const reclaimUnits = node.size;
                targetCol.children.splice(slipIdx, 1);
                const neighbor = targetCol.children[slipIdx] ?? targetCol.children[slipIdx - 1];
                if (neighbor) neighbor.size += reclaimUnits;
            }

            // If the column was converted from a leaf during slip, unwrap it back.
            if (targetWasLeaf && targetCol.children.length === 1) {
                const only = targetCol.children[0];
                if (only.data && !only.children) {
                    targetCol.data = only.data;
                    targetCol.children = undefined;
                    targetCol.flexDirection = FlexDirection.Row;
                }
            }

            // Shrink column back to its original width.
            targetCol.size = Math.max(targetCol.size - originalRowSize, 1);

            // Re-insert node in the Row and clear the slip anchor.
            const rowNode = findParent(model.treeState.rootNode, targetColumnId) ?? model.treeState.rootNode;
            if (rowNode.children) {
                node.size = originalRowSize;
                node.slipMinimize = undefined;
                rowNode._slipAnchor = undefined;
                const idx = Math.min(originalRowIndex, rowNode.children.length);
                rowNode.children.splice(idx, 0, node);
            }
        } else {
            console.warn(`[layoutMinimize] slip restore: target column ${targetColumnId} not found; dropping slip state`);
            node.slipMinimize = undefined;
        }

        _finishToggle(model, nodeId, false);
        return;
    }

    // ── Need parent for minimize and normal restore ───────────────────────────
    const parentNode = findParent(model.treeState.rootNode, nodeId);
    if (!parentNode?.children?.length) return;

    const pixelToSizeRatio = addlProps[parentNode.id]?.pixelToSizeRatio;
    if (!pixelToSizeRatio) return;

    const siblings = parentNode.children;
    const nodeIdx = siblings.findIndex((c) => c.id === nodeId);
    const sibling = siblings[nodeIdx + 1] ?? siblings[nodeIdx - 1];

    // ── Restore: normal path ──────────────────────────────────────────────────
    if (node.minimizedSize !== undefined) {
        if (!sibling) return;
        const headerSizeUnits = (HeaderHeightPx + gapSizePx) * pixelToSizeRatio;
        const reclaimUnits = node.minimizedSize - headerSizeUnits;
        const siblingAfterReclaim = sibling.size - reclaimUnits;
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
        _finishToggle(model, nodeId, false);
        return;
    }

    // ── Minimize ──────────────────────────────────────────────────────────────
    if (!sibling) return;

    if (parentNode.flexDirection === FlexDirection.Row) {
        // Slip path: pane occupies a Row slot — slip header into adjacent column instead
        // of making the pane horizontally narrow.
        if (!_slipMinimize(model, node, nodeIdx, parentNode, sibling, addlProps, gapSizePx)) return;
    } else {
        // Normal path: pane is in a Column — collapse it vertically within the column.
        const headerSizeUnits = (HeaderHeightPx + gapSizePx) * pixelToSizeRatio;
        const freedUnits = node.size - headerSizeUnits;
        if (freedUnits <= 0) return; // already tiny — no-op
        node.minimizedSize = node.size;
        node.size = headerSizeUnits;
        sibling.size += freedUnits;
    }

    _finishToggle(model, nodeId, true);
}

/**
 * Slip a pane from its Row slot into the top of an adjacent column.
 * Returns true on success, false if the tree was not mutated (caller should bail).
 *
 * Sets `_slipAnchor = true` on the parentRow so `balanceNode` skips the
 * single-child-branch hoist rule, keeping the Row(→Column) direction alternation
 * intact while the pane is in the slipped state.
 */
function _slipMinimize(
    model: LayoutModel,
    node: LayoutNode,
    nodeIdx: number,
    parentRow: LayoutNode,
    sibling: LayoutNode,
    addlProps: Record<string, LayoutNodeAdditionalProps>,
    gapSizePx: number,
): boolean {
    const siblingProps = addlProps[sibling.id];
    let targetWasLeaf = false;

    // Convert a plain leaf sibling into a Column container so we can prepend the header.
    if (!sibling.children) {
        targetWasLeaf = true;
        const heightPx = siblingProps?.rect?.height;
        const totalUnits = DefaultNodeSize;
        const headerUnitsEst = heightPx
            ? ((HeaderHeightPx + gapSizePx) * totalUnits) / heightPx
            : totalUnits * 0.05;
        const contentUnits = Math.max(totalUnits - headerUnitsEst, headerUnitsEst);
        const contentNode = newLayoutNode(FlexDirection.Row, contentUnits, undefined, sibling.data);
        sibling.data = undefined;
        sibling.flexDirection = FlexDirection.Column;
        sibling.children = [contentNode];
    }

    // Compute header height in the column's flex-units.
    const colRatio: number | undefined = siblingProps?.pixelToSizeRatio
        ?? (siblingProps?.rect?.height && sibling.children
            ? sibling.children.reduce((s, c) => s + c.size, 0) / siblingProps.rect.height
            : undefined);

    if (colRatio == null) {
        console.warn("[layoutMinimize] slip: cannot determine column ratio, skipping");
        if (targetWasLeaf) {
            const only = sibling.children?.[0];
            if (only?.data) {
                sibling.data = only.data;
                sibling.children = undefined;
                sibling.flexDirection = FlexDirection.Row;
            }
        }
        return false;
    }

    const headerInColUnits = (HeaderHeightPx + gapSizePx) * colRatio;

    // Steal header height from the first column child to keep total column size constant.
    const firstChild = sibling.children![0];
    if (firstChild) {
        firstChild.size = Math.max(firstChild.size - headerInColUnits, headerInColUnits);
    }

    // Record slip state before modifying node.size.
    node.slipMinimize = {
        targetColumnId: sibling.id,
        originalRowSize: node.size,
        originalRowIndex: nodeIdx,
        targetWasLeaf,
    };

    // Remove from Row, give its width to the sibling.
    sibling.size += node.size;
    parentRow.children!.splice(nodeIdx, 1);

    // Mark the Row so balanceNode skips the single-child hoist for this subtree.
    // Without this, balanceNode's flatMap rule would see parentRow(1 child = B branch)
    // and hoist B's children directly into grandparent, scattering them and losing
    // targetColumnId. The Row(→Column) direction alternation stays correct.
    parentRow._slipAnchor = true;

    // Insert at top of the column.
    node.size = headerInColUnits;
    sibling.children!.unshift(node);
    return true;
}

/** Commit tree changes and update the minimized-node-id reactive set. */
function _finishToggle(model: LayoutModel, nodeId: string, minimized: boolean) {
    model.minimizedNodeIds._set((prev) => {
        const next = new Set(prev);
        if (minimized) {
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
 * Scan the loaded tree for nodes that carry a `minimizedSize` or `slipMinimize`
 * field and rebuild the in-memory `minimizedNodeIds` set.
 * Called once during `initializeFromWaveObject`.
 */
export function rebuildMinimizedSet(model: LayoutModel) {
    const ids = new Set<string>();
    function walk(node: typeof model.treeState.rootNode) {
        if (!node) return;
        if (node.minimizedSize !== undefined || node.slipMinimize !== undefined) ids.add(node.id);
        node.children?.forEach(walk);
    }
    walk(model.treeState.rootNode);
    model.minimizedNodeIds._set(ids);
}
