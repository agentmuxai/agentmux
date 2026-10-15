// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { findNode, walkNodes } from "./layoutNode";
import { isNodeLocked, isEffectivelyMinimized, reportLayoutViolations } from "./layoutInvariants";
import type { LayoutModel } from "./layoutModel";
import { DefaultNodeSize, type LayoutNode } from "./types";

/** Height of the block header in CSS pixels (matches --header-height in theme.scss). */
export const HeaderHeightPx = 33;

/**
 * Main-axis width (px) of a minimized pane whose parent is a Row: the pane
 * renders as a compact header chip instead of a full-height narrow strip.
 */
export const MinimizedRowSlotWidthPx = 180;

// Canonical definitions live in layoutInvariants (the doctor validates lock
// invariants, and importing from here would be an import cycle); re-exported
// so existing importers keep working.
export { isNodeLocked, isEffectivelyMinimized };

/**
 * # Minimize is a display mode
 *
 * A minimized pane is a leaf with `minimized: true` — nothing else. Its stored
 * flex `size` is NEVER touched: the renderer (`updateTreeHelper`) derives the
 * header-chip geometry fresh on every pass. Restore is just clearing the
 * flag — the pane's original size is intact by construction, because it
 * never changed.
 *
 * This replaces the previous size-arithmetic model (squeeze `size` to header
 * height + `minimizedSize` restore bookkeeping + "slip" and "column dissolve"
 * as STRUCTURAL TREE SURGERY), which produced four distinct bug classes in
 * two weeks (see `docs/research/RESEARCH_PANE_MINIMIZE_BEST_PRACTICES_2026_07_16.md`
 * §2). The research verdict: every mature system models collapse as either
 * out-of-tree docking or a render-derived display mode; none stores squeezed
 * sizes in the layout tree or splices nodes around to achieve it. Derived
 * state cannot drift, so this model needs no size locks, no snap-back
 * enforcement, and no restore arithmetic.
 *
 * The VISUAL slip/dissolve requirement itself — a minimized pane's header
 * docks onto an adjacent pane, which absorbs its freed space — is a real,
 * deliberately spec'd product requirement
 * (`SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md`,
 * `SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27.md`) that the first cut of
 * this redesign incorrectly deleted along with the buggy mechanism that used
 * to implement it — see
 * `docs/retro/retro-minimize-display-mode-lost-slip-requirement-2026-07-17.md`.
 * It's restored here as pure derived geometry: `layoutGeometry.ts`'s
 * `resolveRowSlipTargets` + the docking pass in `updateTreeHelper` compute,
 * every render, which minimized Row-direction children should render as a
 * header-chip stack overlaid on a sibling instead of claiming their own
 * row-slot — no tree mutation, no restore-context bookkeeping, nothing for
 * it to corrupt.
 *
 * Legacy trees (with `minimizedSize`/`slipMinimize`/`columnDissolve`/
 * `minimizedLockedSize`/`_slipAnchor`) are migrated in place by
 * `rebuildMinimizedSet` at load.
 */

/**
 * Count leaves that are NOT minimized — i.e. panes still showing content. The
 * window must always keep at least one: `minimizeNodeToggle` no-ops when asked
 * to collapse the last expanded pane, and the header hides that pane's
 * minimize button (`NodeModel.canMinimize`).
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
 * Toggle minimize for a leaf node. Minimize sets the display-mode flag;
 * restore clears it. No sizes are read or written.
 */
export function minimizeNodeToggle(model: LayoutModel, nodeId: string) {
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node) return;

    // Only leaves minimize — a branch reaching here indicates a caller bug
    // (and the doctor's I2 would flag the resulting tree).
    if (node.children?.length) {
        console.warn(`[layoutMinimize] toggle on branch ${nodeId} ignored — minimize is leaf-only`);
        return;
    }

    // ── Restore ───────────────────────────────────────────────────────────────
    // Always a restore for a minimized pane, even one that is also maximized:
    // programmatic callers (showBlockInPane, for an editor reuse or a
    // notification) rely on this to make the pane visible. The header's own
    // Minimize on a maximized chip is `collapseMaximizedNode` below.
    if (node.minimized) {
        node.minimized = undefined;
        _finishToggle(model, nodeId, false);
        return;
    }

    // ── Minimize ──────────────────────────────────────────────────────────────
    // The last expanded pane cannot be collapsed — the window must always keep
    // at least one pane showing content. The header already hides the minimize
    // button in this state (NodeModel.canMinimize); this is the authoritative
    // guard for programmatic callers.
    if (countExpandedLeaves(model.treeState.rootNode) <= 1) return;

    // A maximized pane can be minimized directly (SPEC_PANE_CONTROLS_TILED_
    // RESTORE_ICON_AND_MINIMIZE_FROM_MAXIMIZED_2026_10_12 §5). Maximize is a
    // layer over the tree (`magnifiedNodeId`), so the pane still holds its
    // slot and the chip is derived from it: ending the maximize and setting
    // the flag in one commit IS "remember the minimize position". Un-magnify
    // through its own reducer (as removeLeafEmptiedByMove does) so the overlay
    // state stays consistent; setState=false defers the commit to
    // _finishToggle below (one updateTree, one persist), and focused=false
    // leaves focus where minimizing a tiled pane leaves it.
    if (model.treeState.magnifiedNodeId === nodeId) {
        model.magnifyNodeToggle(nodeId, false, false);
    }

    node.minimized = true;
    _finishToggle(model, nodeId, true);
}

/**
 * The header's Minimize while the pane is maximized: end the maximize and leave
 * the pane as a chip, whether or not it already was one (a minimized chip that
 * was maximized is both). Unlike `minimizeNodeToggle` this never restores: that
 * toggle is what programmatic callers use to make a pane visible, so a
 * minimized-and-maximized pane must keep restoring there. Like any minimize it
 * needs one other expanded pane. One commit: one updateTree, one persist.
 */
export function collapseMaximizedNode(model: LayoutModel, nodeId: string) {
    const node = findNode(model.treeState.rootNode, nodeId);
    if (!node || node.children?.length) return;
    if (model.treeState.magnifiedNodeId !== nodeId) return;
    // A pane that is already minimized is not counted as expanded, so it needs one other.
    const otherExpanded = countExpandedLeaves(model.treeState.rootNode) - (isNodeLocked(node) ? 0 : 1);
    if (otherExpanded < 1) return;
    model.magnifyNodeToggle(nodeId, false, false);
    node.minimized = true;
    _finishToggle(model, nodeId, true);
}

/**
 * Commit tree changes. `model.updateTree()` rebuilds `minimizedNodeIds`
 * fresh from the (already-toggled) `node.minimized` flags as part of its own
 * pass — see `layoutGeometry.ts::updateTree` — so there is nothing to set
 * here directly; this function only exists for the doctor-report /
 * persistence side effects around that call.
 */
function _finishToggle(model: LayoutModel, nodeId: string, minimized: boolean) {
    model.updateTree();
    // Layout doctor (issue #2179): validate immediately with toggle attribution.
    reportLayoutViolations(
        model.treeState.rootNode,
        `minimizeToggle:${minimized ? "minimize" : "restore"}:${nodeId.slice(0, 8)}`
    );
    model.localTreeStateAtom._set({ ...model.treeState });
    model.persistToBackend();
}

/**
 * Scan the loaded tree, migrate any legacy minimize state to the display-mode
 * flag, and seed the in-memory `minimizedNodeIds` set for the initial paint
 * (the first `updateTree()` pass — see `layoutGeometry.ts::updateTree` —
 * rebuilds it authoritatively moments later regardless; this just avoids a
 * flash of wrong button state before that first pass runs). Called once
 * during `initializeFromMuxObject`.
 *
 * Migration rules (one-way, in place):
 * - `minimizedSize` (leaf was size-squeezed): restore `size` to the recorded
 *   original, set `minimized`, drop the marker. (Siblings that received the
 *   freed units keep them — a one-time proportional drift, acceptable.)
 * - `slipMinimize` (header slipped into a neighbor column): set `minimized`,
 *   drop the marker. The pane stays where the slip left it — restorable in
 *   place; the original Row-slot context is intentionally not reconstructed.
 * - `columnDissolve` (branch nested into a neighbor): drop the marker. The
 *   structure it built is a valid tree; its children migrate individually.
 * - `minimizedLockedSize` / `_slipAnchor`: dropped (lock layer and balance
 *   carve-out bookkeeping of the legacy model).
 */
export function rebuildMinimizedSet(model: LayoutModel) {
    const root = model.treeState.rootNode;
    const migrated = migrateLegacyMinimize(root);
    if (migrated > 0) {
        console.warn(`[layoutMinimize] migrated ${migrated} legacy minimize field(s) to the display-mode flag`);
    }
    const ids = new Set<string>();
    walkNodes(root, (node) => {
        if (node.minimized && !node.children?.length) ids.add(node.id);
    });
    model.minimizedNodeIds._set(ids);
}

/** The migration half of `rebuildMinimizedSet`, on a bare tree. Returns how
 *  many legacy fields it migrated. srv runs the same migration
 *  (`migrate_legacy_minimize`, crates/srv/src/backend/layout/intent.rs). */
export function migrateLegacyMinimize(root: LayoutNode | undefined): number {
    let migrated = 0;
    // Flex sizes are relative WITHIN a parent. A slipped/dissolved node lives
    // nested in its slip/dissolve TARGET — a different unit space from where
    // its `originalRowSize` was recorded — so restoring that raw number would
    // give it a wildly wrong proportion among its current siblings. Instead,
    // heal its size to a sane share of the parent it actually sits in: the
    // mean of its positive-sized siblings, falling back to DefaultNodeSize.
    const saneShare = (node: LayoutNode, parent: LayoutNode | undefined): number => {
        const sibs = parent?.children?.filter((c) => c !== node && c.size > 0) ?? [];
        if (!sibs.length) return DefaultNodeSize;
        return sibs.reduce((s, c) => s + c.size, 0) / sibs.length;
    };
    function walk(node: LayoutNode, parent: LayoutNode | undefined) {
        if (!node) return;
        const isLeaf = !node.children?.length;
        if (node.minimizedSize !== undefined) {
            // Same parent, same unit space — the recorded original is exact.
            if (isLeaf) {
                node.size = node.minimizedSize;
                node.minimized = true;
            }
            node.minimizedSize = undefined;
            migrated++;
        }
        if (node.slipMinimize !== undefined) {
            // Slip squeezed the leaf to header units inside its TARGET column;
            // heal to a sane share of the column it now sits in.
            node.size = saneShare(node, parent);
            if (isLeaf) node.minimized = true;
            node.slipMinimize = undefined;
            migrated++;
        }
        if (node.columnDissolve !== undefined) {
            // Dissolve set the branch's size to the stolen header total —
            // possibly tiny or even negative (the cascade bug this redesign
            // kills). The size becomes load-bearing again the moment any
            // child leaf restores; heal it to a sane share of its CURRENT
            // (nested) parent.
            node.size = saneShare(node, parent);
            node.columnDissolve = undefined;
            migrated++;
        }
        if (node.minimizedLockedSize !== undefined) {
            node.minimizedLockedSize = undefined;
            migrated++;
        }
        if (node._slipAnchor) {
            node._slipAnchor = undefined;
            migrated++;
        }
        node.children?.forEach((c) => walk(c, node));
    }
    if (root) walk(root, undefined);
    return migrated;
}