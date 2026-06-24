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
 * ### balanceNode hazard
 *
 * `_finishToggle` calls `model.updateTree()` which runs `balanceNode`. That function's
 * `beforeWalkCallback` hoists a child's grandchildren into the parent whenever the
 * child has exactly one grandchild that is itself a branch:
 *   `if (child.children?.length == 1 && child.children[0].children) return child.children[0].children`
 *
 * After a slip on a two-pane Row (A + B), removing A leaves the Row with one child (B,
 * a column with children). `balanceNode` would hoist B's children into the Row's parent,
 * scattering them and invalidating `targetColumnId`. To prevent this we hoist B ourselves
 * before `updateTree` runs and record the Row's restore context in `slipMinimize.promotedRow`.
 */
export function minimizeNodeToggle(model: LayoutModel, nodeId: string) {
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node) return;

    const addlProps = model.getter(model.additionalProps);
    const gapSizePx = model.gapSizePx();

    // ── Restore: slip path ────────────────────────────────────────────────────
    if (node.slipMinimize !== undefined) {
        const { targetColumnId, originalRowSize, originalRowIndex, targetWasLeaf, promotedRow } = node.slipMinimize;

        const targetCol = findNode(model.treeState.rootNode, targetColumnId);
        if (targetCol?.children) {
            // Remove the slipped header from the column; return its size to its neighbor.
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

            if (promotedRow) {
                // The Row was hoisted during slip: targetCol is now a direct child of
                // the grandparent. Re-wrap A and B in a new Row and replace targetCol
                // in the grandparent.
                const grandparent = findNode(model.treeState.rootNode, promotedRow.rowParentId);
                if (grandparent?.children) {
                    const bIdx = grandparent.children.findIndex((c) => c.id === targetColumnId);
                    if (bIdx !== -1) {
                        // Restore B's original Row-width and A's size.
                        targetCol.size = promotedRow.originalTargetRowSize;
                        node.size = originalRowSize;
                        node.slipMinimize = undefined;
                        // Re-create the Row in the original slot order.
                        const rowChildren: LayoutNode[] =
                            originalRowIndex === 0 ? [node, targetCol] : [targetCol, node];
                        const newRow = newLayoutNode(FlexDirection.Row, promotedRow.rowSize, rowChildren);
                        grandparent.children.splice(bIdx, 1, newRow);
                    }
                }
            } else {
                // Normal restore: Row still exists. Shrink B back to its original width
                // and re-insert A at its original index.
                targetCol.size = Math.max(targetCol.size - originalRowSize, 1);
                const rowNode = findParent(model.treeState.rootNode, targetColumnId) ?? model.treeState.rootNode;
                if (rowNode.children) {
                    node.size = originalRowSize;
                    node.slipMinimize = undefined;
                    const idx = Math.min(originalRowIndex, rowNode.children.length);
                    rowNode.children.splice(idx, 0, node);
                }
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
 * When the Row has exactly two children (this pane + the target), removing this pane
 * would leave a single-child Row that `balanceNode` would hoist — scattering the
 * target column's children and losing `targetColumnId`. We detect this and hoist
 * the target ourselves, recording the context in `slipMinimize.promotedRow`.
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

    // Capture B's original Row-width before mutating sizes (needed for promotedRow restore).
    const originalTargetRowSize = sibling.size;

    // Give A's Row-width to B, then remove A from the Row.
    sibling.size += node.size;
    parentRow.children!.splice(nodeIdx, 1);

    // Detect single-child Row and hoist B ourselves to prevent balanceNode from
    // scattering B's children (which would invalidate targetColumnId on restore).
    let promotedRow: NonNullable<LayoutNode["slipMinimize"]>["promotedRow"] | undefined;
    if (parentRow.children!.length === 1) {
        const grandparent = findParent(model.treeState.rootNode, parentRow.id);
        if (grandparent?.children) {
            const rowIdx = grandparent.children.findIndex((c) => c.id === parentRow.id);
            if (rowIdx !== -1) {
                promotedRow = {
                    rowParentId: grandparent.id,
                    rowIdx,
                    rowSize: parentRow.size,
                    originalTargetRowSize,
                };
                // Hoist: give B the Row's Column-slot size and replace the Row in grandparent.
                sibling.size = parentRow.size;
                grandparent.children.splice(rowIdx, 1, sibling);
            }
        }
    }

    // Record slip state before modifying node.size.
    node.slipMinimize = {
        targetColumnId: sibling.id,
        originalRowSize: node.size,
        originalRowIndex: nodeIdx,
        targetWasLeaf,
        promotedRow,
    };

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
