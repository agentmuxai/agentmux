// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { findNode } from "./layoutNode";
import { isNodeLocked, isEffectivelyMinimized, reportLayoutViolations } from "./layoutInvariants";
import type { LayoutModel } from "./layoutModel";
import type { LayoutNode } from "./types";

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
 * header-chip geometry fresh on every pass (header height in a Column parent,
 * a fixed-width chip in a Row parent, and a fully-minimized subtree renders as
 * a stacked strip of chips). Restore is just clearing the flag — the pane's
 * original size is intact by construction, because it never changed.
 *
 * This replaces the previous size-arithmetic model (squeeze `size` to header
 * height + `minimizedSize` restore bookkeeping + "slip" and "column dissolve"
 * structural surgery), which produced four distinct bug classes in two weeks
 * (see `docs/research/RESEARCH_PANE_MINIMIZE_BEST_PRACTICES_2026_07_16.md` §2).
 * The research verdict: every mature system models collapse as either
 * out-of-tree docking or a render-derived display mode (i3 stacked/tabbed,
 * maximize toggles); none stores squeezed sizes in the layout tree. Derived
 * state cannot drift, so this model needs no size locks, no snap-back
 * enforcement, and no restore arithmetic.
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

    node.minimized = true;
    _finishToggle(model, nodeId, true);
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
 * flag, and rebuild the in-memory `minimizedNodeIds` set. Called once during
 * `initializeFromWaveObject`.
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
    const ids = new Set<string>();
    let migrated = 0;
    const root = model.treeState.rootNode;
    function walk(node: LayoutNode) {
        if (!node) return;
        const isLeaf = !node.children?.length;
        if (node.minimizedSize !== undefined) {
            if (isLeaf) {
                node.size = node.minimizedSize;
                node.minimized = true;
            }
            node.minimizedSize = undefined;
            migrated++;
        }
        if (node.slipMinimize !== undefined) {
            if (isLeaf) node.minimized = true;
            node.slipMinimize = undefined;
            migrated++;
        }
        if (node.columnDissolve !== undefined) {
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
        if (node.minimized && isLeaf) ids.add(node.id);
        node.children?.forEach(walk);
    }
    if (root) walk(root);
    if (migrated > 0) {
        console.warn(`[layoutMinimize] migrated ${migrated} legacy minimize field(s) to the display-mode flag`);
    }
    model.minimizedNodeIds._set(ids);
}