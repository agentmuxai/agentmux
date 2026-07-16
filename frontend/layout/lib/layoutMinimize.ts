// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


import { newLayoutNode } from "./layoutNode";
import { findNode, findParent } from "./layoutNode";
import { isNodeLocked, reportLayoutViolations } from "./layoutInvariants";
import type { LayoutModel } from "./layoutModel";
import { DefaultNodeSize, FlexDirection, type LayoutNodeAdditionalProps, type LayoutNode } from "./types";

/** Height of the block header in CSS pixels (matches --header-height in theme.scss). */
export const HeaderHeightPx = 33;

// Canonical definition lives in layoutInvariants (the doctor validates lock
// invariants, and importing from here would be an import cycle); re-exported
// so existing importers keep working.
export { isNodeLocked };

/**
 * Count leaves that are NOT minimize-locked — i.e. panes still showing
 * content. The window must always keep at least one: `minimizeNodeToggle`
 * no-ops when asked to collapse the last expanded pane, and the header hides
 * that pane's minimize button (`NodeModel.canMinimize`).
 */
export function countExpandedLeaves(root: LayoutNode | undefined): number {
    if (!root) return 0;
    let count = 0;
    function walk(node: LayoutNode) {
        if (!node.children?.length) {
            if (node.data !== undefined && !isNodeLocked(node)) count++;
            return;
        }
        node.children.forEach(walk);
    }
    walk(root);
    return count;
}

/**
 * Write-point enforcement of the minimize lock: snap every locked node's `size`
 * back to its recorded `minimizedLockedSize`, returning the delta to the nearest
 * unlocked sibling (next preferred, then previous) so the parent's unit budget is
 * conserved. Runs on every `updateTree` pass — the same choke point as
 * `balanceNode` — so a locked size survives any writer that bypasses the reducer
 * guards (stale tree pushes, direct mutations). Returns the number of snapped nodes.
 *
 * If every sibling is locked (inside a dissolved column) the delta is dropped:
 * flex sizes are relative within the parent, so the locked nodes' shares stay
 * proportionally correct.
 */
export function enforceMinimizedLocks(root: LayoutNode | undefined): number {
    let snapped = 0;
    function walk(node: LayoutNode) {
        if (!node.children) return;
        node.children.forEach((child, i) => {
            if (isNodeLocked(child) && child.minimizedLockedSize !== undefined) {
                const delta = child.size - child.minimizedLockedSize;
                if (Math.abs(delta) > 1e-4) {
                    child.size = child.minimizedLockedSize;
                    const beneficiary =
                        node.children!.slice(i + 1).find((c) => !isNodeLocked(c)) ??
                        node.children!.slice(0, i).reverse().find((c) => !isNodeLocked(c));
                    // Floor at 1 flex unit (same floor as the slip/undissolve
                    // restore paths): a tampered size BELOW the lock makes the
                    // delta negative, and the repayment must not drive the
                    // beneficiary's size to zero or negative.
                    if (beneficiary) beneficiary.size = Math.max(beneficiary.size + delta, 1);
                    snapped++;
                }
            }
            walk(child);
        });
    }
    if (root) walk(root);
    return snapped;
}

/**
 * Toggle minimize for a given leaf node.
 *
 * There are three minimize paths:
 *
 * **Column parent (normal path):** The node collapses vertically to its header bar.
 * Freed space is given to the adjacent sibling. `node.minimizedSize` stores the
 * original height for restore. After collapsing, if ALL children of the parent
 * column are now minimized, the column itself dissolves (see below).
 *
 * **Row parent (slip path):** Shrinking the node's Row-slot size would make it
 * horizontally narrow (thin strip) rather than vertically collapsed. Instead the
 * pane "slips" its header to the top of the nearest adjacent column, giving its
 * full width to that column. `node.slipMinimize` stores restore context.
 *
 * **Column dissolve:** When every leaf in a Column is minimized, the column is
 * pulled out of the root Row and re-inserted as a branch at the top of the adjacent
 * column. The column's Row-slot width is given to that adjacent column. On restore,
 * clicking any child's minimize button undissolves the column first, returning it to
 * its original Row position with all children still minimized. `colNode.columnDissolve`
 * stores restore context.
 *
 * ### balanceNode and the _slipAnchor invariant
 *
 * After a slip or dissolve, the parentRow may have exactly one child. Normally
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
                node.minimizedLockedSize = undefined;
                rowNode._slipAnchor = undefined;
                const idx = Math.min(originalRowIndex, rowNode.children.length);
                rowNode.children.splice(idx, 0, node);
            }
        } else {
            console.warn(`[layoutMinimize] slip restore: target column ${targetColumnId} not found; dropping slip state`);
            node.slipMinimize = undefined;
            node.minimizedLockedSize = undefined;
        }

        _finishToggle(model, nodeId, false);
        return;
    }

    // ── Pre-check: parent column dissolved — undissolve before toggling ───────
    // A node inside a dissolved column has `minimizedSize` set and lives inside a
    // column that itself has `columnDissolve`. Clicking restore on that node first
    // undissolves the parent column (returns it to its Row slot), then falls through
    // to the normal restore path so both the undissolve and the individual pane
    // restore happen in a single click.
    //
    // In a cascade (A→B→C, 3-deep): colA dissolves into colB, then colB dissolves
    // into colC. When the user restores a pane in colA, the immediate parent (colA)
    // has columnDissolve set, but colA's targetColumnId (colB) is itself still
    // dissolved inside colC. We must undissolve ancestor columns from outermost to
    // innermost before undissolving the immediate parent, otherwise _undissolveColumn
    // cannot find the correct Row insertion point and colB.size underflows.
    const parentForDissolveCheck = findParent(model.treeState.rootNode, nodeId);
    if (parentForDissolveCheck?.columnDissolve !== undefined) {
        // Collect all dissolved ancestors of parentForDissolveCheck from innermost
        // to outermost by walking up via the columnDissolve.targetColumnId chain.
        const dissolvedAncestors: LayoutNode[] = [];
        let cursor: LayoutNode | undefined = parentForDissolveCheck;
        while (cursor?.columnDissolve !== undefined) {
            const targetCol = findNode(model.treeState.rootNode, cursor.columnDissolve.targetColumnId);
            if (targetCol?.columnDissolve !== undefined) {
                dissolvedAncestors.push(targetCol);
            }
            cursor = targetCol;
        }
        // Undissolve from outermost to innermost so each column is returned to its
        // Row slot before its inner neighbour tries to use it as an insertion target.
        for (const ancestor of dissolvedAncestors.reverse()) {
            _undissolveColumn(model, ancestor);
        }
        // Now undissolve the immediate parent of the clicked pane.
        _undissolveColumn(model, parentForDissolveCheck);
        // Fall through — node still has minimizedSize set; normal restore runs below.
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
        node.minimizedLockedSize = undefined;
        _finishToggle(model, nodeId, false);
        return;
    }

    // ── Minimize ──────────────────────────────────────────────────────────────
    if (!sibling) return;

    // The last expanded pane cannot be collapsed — the window must always keep
    // at least one pane showing content (an all-headers window is dead space
    // with nothing restorable in view). The header already hides the minimize
    // button in this state (NodeModel.canMinimize); this is the authoritative
    // guard for programmatic callers.
    if (countExpandedLeaves(model.treeState.rootNode) <= 1) return;

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
        node.minimizedLockedSize = headerSizeUnits;
        sibling.size += freedUnits;

        // If all children of the column are now minimized (leaves via minimizedSize/
        // slipMinimize, or previously dissolved sub-columns via columnDissolve),
        // dissolve this column into the adjacent Row sibling.
        const allCollapsed = parentNode.children.every(
            (c) => c.minimizedSize !== undefined || c.slipMinimize !== undefined || c.columnDissolve !== undefined
        );
        if (allCollapsed) {
            _dissolveColumn(model, parentNode, addlProps, gapSizePx);
        }
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
    node.minimizedLockedSize = headerInColUnits;
    sibling.children!.unshift(node);
    return true;
}

/**
 * Dissolve a fully-collapsed column into an adjacent column.
 *
 * Called after the last leaf in `colNode` is minimized. Removes `colNode` from its
 * Row slot, gives its width to the adjacent sibling column, and re-inserts `colNode`
 * as a branch at the top of that sibling. The column's leaf children retain their
 * individual `minimizedSize` values.
 *
 * Returns true on success; false if bailing without mutating the tree (no Row parent,
 * no adjacent sibling, or column-ratio unavailable).
 */
function _dissolveColumn(
    model: LayoutModel,
    colNode: LayoutNode,
    addlProps: Record<string, LayoutNodeAdditionalProps>,
    gapSizePx: number,
): boolean {
    const rowNode = findParent(model.treeState.rootNode, colNode.id);
    if (!rowNode?.children?.length || rowNode.flexDirection !== FlexDirection.Row) return false;

    const nodeIdx = rowNode.children.findIndex((c) => c.id === colNode.id);
    const sibling = rowNode.children[nodeIdx + 1] ?? rowNode.children[nodeIdx - 1];
    if (!sibling) return false;

    // Don't dissolve into a column that is itself dissolved (it's nested elsewhere).
    if (sibling.columnDissolve !== undefined) return false;

    const siblingProps = addlProps[sibling.id];
    let targetWasLeaf = false;

    // Convert a plain leaf sibling into a Column container so we can prepend the branch.
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

    // Compute dissolved column's height in the sibling column's flex-unit space.
    const colRatio: number | undefined = siblingProps?.pixelToSizeRatio
        ?? (siblingProps?.rect?.height && sibling.children
            ? sibling.children.reduce((s, c) => s + c.size, 0) / siblingProps.rect.height
            : undefined);

    if (colRatio == null) {
        console.warn("[layoutMinimize] dissolve: cannot determine column ratio, skipping");
        if (targetWasLeaf) {
            const only = sibling.children?.[0];
            if (only) {
                sibling.data = only.data;
                sibling.children = undefined;
                sibling.flexDirection = FlexDirection.Row;
            }
        }
        return false;
    }

    // One header per leaf child — count recursively: a dissolved sub-column
    // (with columnDissolve set) represents multiple collapsed headers and should
    // count as its children.length, not 1, so cascade dissolve heights are correct.
    const leafCount = colNode.children!.reduce((sum, c) =>
        sum + (c.columnDissolve !== undefined && c.children ? c.children.length : 1), 0);
    const totalHeaderUnits = leafCount * (HeaderHeightPx + gapSizePx) * colRatio;

    // Steal from sibling's first child so sibling's total flex-size stays constant.
    // Track actualStolen separately: if firstChild.size is clamped to the floor,
    // less than totalHeaderUnits is stolen, so colNode.size must reflect that.
    const firstChild = sibling.children![0];
    let actualStolen = totalHeaderUnits;
    if (firstChild) {
        const floorUnits = (HeaderHeightPx + gapSizePx) * colRatio;
        const prevSize = firstChild.size;
        firstChild.size = Math.max(firstChild.size - totalHeaderUnits, floorUnits);
        actualStolen = prevSize - firstChild.size;
    }

    // Record dissolve context.
    colNode.columnDissolve = {
        targetColumnId: sibling.id,
        originalRowSize: colNode.size,
        originalRowIndex: nodeIdx,
        targetWasLeaf,
    };

    // Give the dissolved column's Row-slot width to the sibling; remove from Row.
    sibling.size += colNode.size;
    rowNode.children.splice(nodeIdx, 1);
    rowNode._slipAnchor = true;

    // Insert the dissolved column branch at the top of the sibling column.
    // Use actualStolen (not totalHeaderUnits) to keep the size budget honest
    // when firstChild.size was clamped to the floor value.
    colNode.size = actualStolen;
    colNode.minimizedLockedSize = actualStolen;
    sibling.children!.unshift(colNode);

    return true;
}

/**
 * Walk up the ancestor chain from `nodeId` and return the first ancestor whose
 * `flexDirection` is `FlexDirection.Row`.  Returns `undefined` if none is found
 * (e.g. the node is already at the root level with no Row ancestor).
 *
 * Used by `_undissolveColumn` to locate the correct Row insertion point in
 * cascade-dissolve scenarios where the host column is itself nested inside
 * another dissolved column.
 */
function _findRowAncestor(root: LayoutNode, nodeId: string): LayoutNode | undefined {
    const parent = findParent(root, nodeId);
    if (!parent) return undefined;
    if (parent.flexDirection === FlexDirection.Row) return parent;
    return _findRowAncestor(root, parent.id);
}

/**
 * Restore a dissolved column to its original Row slot.
 *
 * Removes `colNode` from its host column, shrinks the host back by the original
 * width, and re-inserts `colNode` into the root Row at its original index. The
 * column's children retain their `minimizedSize` values — they remain visually
 * collapsed within the restored column.
 */
function _undissolveColumn(model: LayoutModel, colNode: LayoutNode): void {
    const { targetColumnId, originalRowSize, originalRowIndex, targetWasLeaf } = colNode.columnDissolve!;

    const targetCol = findNode(model.treeState.rootNode, targetColumnId);
    if (!targetCol?.children) {
        console.warn(`[layoutMinimize] undissolve: target column ${targetColumnId} not found`);
        return;
    }
    // Clear dissolve context only after confirming targetCol exists — clearing
    // it before the check would permanently lose restore context on a missing target.
    colNode.columnDissolve = undefined;
    colNode.minimizedLockedSize = undefined;

    // Remove colNode from the host column; give its height back to the neighbor.
    const slipIdx = targetCol.children.findIndex((c) => c.id === colNode.id);
    if (slipIdx !== -1) {
        const reclaimUnits = colNode.size;
        targetCol.children.splice(slipIdx, 1);
        const neighbor = targetCol.children[slipIdx] ?? targetCol.children[slipIdx - 1];
        if (neighbor) neighbor.size += reclaimUnits;
    }

    // If the target was originally a leaf that got wrapped, unwrap it.
    if (targetWasLeaf && targetCol.children.length === 1) {
        const only = targetCol.children[0];
        if (!only.children) {
            targetCol.data = only.data;
            targetCol.children = undefined;
            targetCol.flexDirection = FlexDirection.Row;
        }
    }

    // Shrink the host column back to its pre-dissolve width.
    targetCol.size = Math.max(targetCol.size - originalRowSize, 1);

    // Re-insert colNode into the Row at its original index.
    // IMPORTANT: In a cascade-dissolve scenario, targetCol may itself be
    // nested inside another column (dissolved into a third column).
    // findParent(root, targetColumnId) would return that outer column rather
    // than the Row — causing colNode to be spliced in as a Column-in-Column
    // instead of going back to its Row slot. We must walk up the ancestor
    // chain from targetCol until we reach a Row-direction node.
    const rowNode = _findRowAncestor(model.treeState.rootNode, targetColumnId) ?? model.treeState.rootNode;
    if (rowNode.children) {
        colNode.size = originalRowSize;
        // Only clear _slipAnchor if no remaining sibling has a concurrent slipMinimize
        // that also depends on it — undissolving one column must not remove an anchor
        // that a slip-minimized pane on the same row still needs.
        const stillNeedsAnchor = rowNode.children.some((c) => c.slipMinimize !== undefined);
        if (!stillNeedsAnchor) {
            rowNode._slipAnchor = undefined;
        }
        const idx = Math.min(originalRowIndex, rowNode.children.length);
        rowNode.children.splice(idx, 0, colNode);
    }
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
    // Layout doctor (issue #2179): a minimize/restore toggle is the highest-
    // risk mutation in the layout system (slip / dissolve / undissolve all
    // restructure the tree). Validate immediately with toggle attribution —
    // updateTree above also validates, but this context names the culprit.
    reportLayoutViolations(
        model.treeState.rootNode,
        `minimizeToggle:${minimized ? "minimize" : "restore"}:${nodeId.slice(0, 8)}`
    );
    model.localTreeStateAtom._set({ ...model.treeState });
    model.persistToBackend();
}

/**
 * Scan the loaded tree for nodes that carry a `minimizedSize` or `slipMinimize`
 * field and rebuild the in-memory `minimizedNodeIds` set. Also restores `_slipAnchor`
 * on any Row that owns the host column of a dissolved branch.
 * Called once during `initializeFromWaveObject`.
 */
export function rebuildMinimizedSet(model: LayoutModel) {
    const ids = new Set<string>();
    const root = model.treeState.rootNode;
    function walk(node: typeof root) {
        if (!node) return;
        if (node.minimizedSize !== undefined || node.slipMinimize !== undefined) ids.add(node.id);
        if (node.columnDissolve !== undefined) {
            // Restore _slipAnchor on the Row that owns the dissolved column's host.
            const targetRow = findParent(root, node.columnDissolve.targetColumnId);
            if (targetRow) targetRow._slipAnchor = true;
        }
        node.children?.forEach(walk);
    }
    walk(root);
    model.minimizedNodeIds._set(ids);
}
