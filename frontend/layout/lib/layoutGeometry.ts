// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { batch } from "solid-js";
import { balanceNode, walkNodes } from "./layoutNode";
import { HeaderHeightPx, MinimizedRowSlotWidthPx } from "./layoutMinimize";
import { isEffectivelyMinimized, reportLayoutViolations } from "./layoutInvariants";
import {
    FlexDirection,
    LayoutNode,
    LayoutNodeAdditionalProps,
    LayoutTreeActionType,
    LayoutTreeResizeNodeAction,
    ResizeHandleProps,
} from "./types";
import { setTransform } from "./utils";
import type { LayoutModel } from "./layoutModel";

export interface MainAxisAllocation {
    /** Main-axis pixels per child, index-aligned with the children array. */
    px: number[];
    /**
     * Flex-units-per-pixel for the EXPANDED children (minimized chips take
     * fixed pixels off the top first). This is what resize-drag math uses to
     * convert pointer deltas into flex units, so it must describe only the
     * space the flex solver actually distributes.
     */
    pixelToSizeRatio: number;
}

/** Leaf count of a subtree — one header chip per leaf when fully minimized. */
function countLeafPanes(node: LayoutNode): number {
    if (!node.children?.length) return 1;
    return node.children.reduce((s, c) => s + countLeafPanes(c), 0);
}

/**
 * Fixed main-axis pixels for a minimized child: a header-height strip per
 * stacked chip in a Column parent, a compact fixed-width chip in a Row parent.
 * Derived fresh every pass — minimize never writes sizes (see layoutMinimize).
 *
 * Gap compensation: each tile's rendered box is inset by `gapSizePx`
 * downstream (`innerRect` computes `calc(size - gapSizePx)` in
 * layoutNodeModels.ts), and the header has a FIXED --header-height in
 * block.scss — so the slot allocation must be header + gap for the visible
 * box to come out exactly header-sized instead of clipping.
 */
function minimizedFixedPx(node: LayoutNode, parentIsRow: boolean, gapPx: number): number {
    if (parentIsRow) return MinimizedRowSlotWidthPx + gapPx;
    return countLeafPanes(node) * (HeaderHeightPx + gapPx);
}

/**
 * Cross-axis (height) a minimized child needs when its parent is a Row: one
 * header height per leaf in its subtree, stacked. A single minimized leaf
 * needs one header height; a fully-minimized BRANCH holding N leaves needs
 * N header-heights, because its own recursive Column layout will stack N
 * chips inside whatever cross-axis space its parent gives it here — giving
 * it less produces overlapping/clipped chips, giving it more produces dead
 * space below them (the bug this function exists to prevent). Same formula
 * as `minimizedFixedPx`'s Column-parent branch — a stack's required extent
 * doesn't depend on which axis it's being measured for. Pure; exported for
 * unit tests.
 */
export function minimizedCrossAxisPx(child: LayoutNode, gapPx: number): number {
    return minimizedFixedPx(child, false, gapPx);
}

/**
 * Derive each child's main-axis pixels: minimized children get fixed
 * chip-sized allocations (scaled down proportionally if the container is too
 * small to fit them all), and the remaining pixels are split among expanded
 * children proportionally to their stored flex sizes — which are never
 * mutated by minimize. `slipChildIds` (Row direction only — see
 * `resolveRowSlipTargets`) marks children that dock onto a sibling instead
 * of claiming a slot at all: they contribute ZERO pixels here (excluded from
 * both the fixed-chip and flex pools), so their would-be space flows
 * entirely to the remaining flex children — the width-reclaim half of the
 * slip requirement. The height/position half (docking the chip visually
 * onto the target) is a separate pass in `updateTreeHelper`. Pure; exported
 * for unit tests.
 */
export function computeMainAxisAllocation(
    children: LayoutNode[],
    nodeIsRow: boolean,
    nodePixels: number,
    getSize: (n: LayoutNode) => number,
    gapPx = 0,
    slipChildIds?: Set<string>
): MainAxisAllocation {
    const isSlip = (c: LayoutNode) => slipChildIds?.has(c.id) ?? false;
    const fixed = children.map((c) =>
        isSlip(c) ? 0 : isEffectivelyMinimized(c) ? minimizedFixedPx(c, nodeIsRow, gapPx) : 0
    );
    const fixedTotal = fixed.reduce((a, b) => a + b, 0);
    const scale = fixedTotal > nodePixels && fixedTotal > 0 ? nodePixels / fixedTotal : 1;
    const remainingPx = Math.max(nodePixels - fixedTotal * scale, 0);
    const flexTotal = children.reduce((s, c, i) => (fixed[i] || isSlip(c) ? s : s + getSize(c)), 0);
    const pixelToSizeRatio =
        flexTotal > 0 && remainingPx > 0
            ? flexTotal / remainingPx
            : children.reduce((s, c) => s + getSize(c), 0) / Math.max(nodePixels, 1);
    const px = children.map((c, i) =>
        isSlip(c) ? 0 : fixed[i] ? fixed[i] * scale : flexTotal > 0 ? getSize(c) / pixelToSizeRatio : 0
    );
    return { px, pixelToSizeRatio };
}

/**
 * For each child of a Row node, resolve which sibling it should visually
 * dock onto instead of claiming its own row-slot: the nearest
 * non-minimized sibling, right-preferred, else left — generalizing
 * `SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md`'s per-pane "slip" and
 * `SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27.md`'s cascading
 * full-column dissolve into one rule via `isEffectivelyMinimized` (a
 * minimized leaf and a fully-collapsed branch are treated identically —
 * both need somewhere to dock). Unlike the original tree-surgery
 * implementation this never mutates the tree or needs restore-context
 * bookkeeping: it's recomputed fresh every render pass from nothing but the
 * `minimized` flag, so there is nothing for it to corrupt (see
 * `docs/retro/retro-minimize-display-mode-lost-slip-requirement-2026-07-17.md`).
 *
 * A child gets no entry (falls back to its own fixed-width chip slot via
 * `computeMainAxisAllocation`'s ordinary minimized-but-not-slipping path)
 * when it isn't effectively minimized, or when every sibling in the row is
 * ALSO effectively minimized (no valid anchor to dock onto — can happen
 * locally even though the last-expanded-pane guard prevents it globally,
 * since that guard counts leaves across the whole tree, not per row).
 * Multiple children can resolve to the same target; callers stack them.
 * Pure; exported for unit tests.
 */
export function resolveRowSlipTargets(children: LayoutNode[]): Map<string, LayoutNode> {
    const targets = new Map<string, LayoutNode>();
    const minimized = children.map(isEffectivelyMinimized);
    for (let i = 0; i < children.length; i++) {
        if (!minimized[i]) continue;
        let target: LayoutNode | undefined;
        for (let j = i + 1; j < children.length; j++) {
            if (!minimized[j]) {
                target = children[j];
                break;
            }
        }
        if (!target) {
            for (let j = i - 1; j >= 0; j--) {
                if (!minimized[j]) {
                    target = children[j];
                    break;
                }
            }
        }
        if (target) targets.set(children[i].id, target);
    }
    return targets;
}

/**
 * Recursively walks the tree to find leaf nodes, update the resize handles, and compute additional properties for each node.
 * @param model The LayoutModel instance.
 * @param balanceTree Whether the tree should also be balanced as it is walked. Defaults to true.
 */
export function updateTree(model: LayoutModel, balanceTree = true) {
    if (model.displayContainerRef.current) {
        const newLeafs: LayoutNode[] = [];
        const newAdditionalProps = {};

        const pendingAction = model.getter(model.pendingTreeAction.currentValueAtom);
        const resizeAction =
            pendingAction?.type === LayoutTreeActionType.ResizeNode
                ? (pendingAction as LayoutTreeResizeNodeAction)
                : null;
        const resizeHandleSizePx = model.getter(model.resizeHandleSizePx);

        const boundingRect = model.getBoundingRect();

        const magnifiedNodeSize = model.getter(model.magnifiedNodeSizeAtom) ?? 0.8;

        const callback = (node: LayoutNode) =>
            updateTreeHelper(
                model,
                node,
                newAdditionalProps,
                newLeafs,
                resizeHandleSizePx,
                magnifiedNodeSize,
                boundingRect,
                resizeAction
            );
        if (balanceTree) {
            // Minimize is a display mode: geometry for minimized panes is
            // derived inside updateTreeHelper each pass, and stored sizes are
            // never touched — so there are no size locks to enforce here
            // anymore (see layoutMinimize.ts header).
            model.treeState.rootNode = balanceNode(model.treeState.rootNode, callback);
            // Layout doctor (issue #2179): observe the post-normalization tree
            // and log loudly if any structural invariant is broken, so a
            // corruption is attributed at the pass that produced it instead of
            // reconstructed later from db_layout archaeology.
            reportLayoutViolations(model.treeState.rootNode, "updateTree");
        } else walkNodes(model.treeState.rootNode, callback);

        // Process ephemeral node, if present.
        const ephemeralNode = model.getter(model.ephemeralNode);
        if (ephemeralNode) {
            model.updateEphemeralNodeProps(
                ephemeralNode,
                newAdditionalProps,
                newLeafs,
                magnifiedNodeSize,
                boundingRect
            );
        }

        model.treeState.leafOrder = getLeafOrder(newLeafs, newAdditionalProps);
        model.validateFocusedNode(model.treeState.leafOrder);
        model.validateMagnifiedNode(model.treeState.leafOrder, newAdditionalProps);
        model.cleanupNodeModels(model.treeState.leafOrder);
        const sortedLeafs = newLeafs.sort((a, b) => a.id.localeCompare(b.id));

        // Rebuild minimizedNodeIds fresh from the actual `minimized` flags on
        // this pass's leaves — the SAME walk `newLeafs` already comes from,
        // so this is free. minimizedNodeIds only exists to drive the
        // minimize/restore BUTTON ICON (isMinimized/canMinimize in
        // layoutNodeModels.ts); it is not read by any geometry code, which
        // reads `isEffectivelyMinimized`/`node.minimized` directly. Deriving
        // it here instead of incrementally maintaining it (the previous
        // design: _finishToggle add/remove by id) makes it structurally
        // impossible for the button to disagree with the tree — the exact
        // failure mode hit when a minimized leaf got promoted to a branch by
        // an unguarded insert (addIntermediateNode moves `data` into a fresh
        // intermediate child that inherits the leaf's id, but nothing moved
        // `minimized` off the outer node) and the OLD incrementally-tracked
        // set kept referencing that inherited id after it was no longer
        // minimized, showing "Restore" on an expanded pane.
        const newMinimizedIds = new Set(newLeafs.filter((l) => l.minimized).map((l) => l.id));
        batch(() => {
            model.setter(model.leafs, sortedLeafs);
            model.setter(model.leafOrder, model.treeState.leafOrder);
            model.setter(model.additionalProps, newAdditionalProps);
            model.minimizedNodeIds._set(newMinimizedIds);
        });
    }
}

/**
 * Per-node callback that is invoked recursively to find leaf nodes, update the resize handles, and compute additional properties.
 * @param model The LayoutModel instance.
 * @param node The node for which to update the resize handles and additional properties.
 * @param additionalPropsMap The new map that will contain the updated additional properties for all nodes in the tree.
 * @param leafs The new list that will contain all the leaf nodes in the tree.
 * @param resizeHandleSizePx The resize handle size in CSS pixels.
 * @param magnifiedNodeSizePct The magnified node size as a percentage.
 * @param boundingRect The bounding rect of the layout container.
 * @param resizeAction The pending resize action, if any.
 */
function updateTreeHelper(
    model: LayoutModel,
    node: LayoutNode,
    additionalPropsMap: Record<string, LayoutNodeAdditionalProps>,
    leafs: LayoutNode[],
    resizeHandleSizePx: number,
    magnifiedNodeSizePct: number,
    boundingRect: Dimensions,
    resizeAction?: LayoutTreeResizeNodeAction
) {
    if (!node.children?.length) {
        leafs.push(node);
        let addlProps = additionalPropsMap[node.id];

        // BUG FIX: When a single leaf is the root node, it won't have additionalProps
        // because those are normally set by the parent node processing its children.
        // We need to create additionalProps for the root leaf using the full boundingRect.
        if (!addlProps && node.id === model.treeState.rootNode?.id) {
            const transform = setTransform(boundingRect);
            addlProps = {
                rect: boundingRect,
                transform,
                treeKey: "0",
            };
            additionalPropsMap[node.id] = addlProps;
        }

        if (addlProps) {
            // Magnified pane is now rendered in a separate overlay container (MagnifiedPaneOverlay),
            // so we no longer override its transform/z-index here. The tile-node slot stays at its
            // original position but is hidden via CSS (tile-hidden class).
            if (model.lastMagnifiedNodeId === node.id) {
                addlProps.transform.zIndex = "var(--zindex-layout-last-magnified-node)";
            } else if (model.lastEphemeralNodeId === node.id) {
                addlProps.transform.zIndex = "var(--zindex-layout-last-ephemeral-node)";
            }
        }
        return;
    }

    function getNodeSize(node: LayoutNode) {
        return resizeAction?.resizeOperations.find((op) => op.nodeId === node.id)?.size ?? node.size;
    }

    const additionalProps: LayoutNodeAdditionalProps = additionalPropsMap.hasOwnProperty(node.id)
        ? additionalPropsMap[node.id]
        : { treeKey: "0" };

    const nodeRect: Dimensions = node.id === model.treeState.rootNode.id ? boundingRect : additionalProps.rect;
    const nodeIsRow = node.flexDirection === FlexDirection.Row;
    const nodePixels = nodeIsRow ? nodeRect.width : nodeRect.height;
    const gapPx = model.gapSizePx();

    // Row-only: resolve which children dock onto a sibling instead of
    // claiming their own row-slot. Column direction never slips — a
    // minimized Column child already renders correctly in place (header
    // height, stacked with its siblings, no dead space). Restored per
    // docs/retro/retro-minimize-display-mode-lost-slip-requirement-2026-07-17.md
    // — SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md /
    // SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27.md's original requirement,
    // reimplemented as derived geometry instead of tree surgery.
    const slipTargets = nodeIsRow ? resolveRowSlipTargets(node.children) : new Map<string, LayoutNode>();
    const slipChildIds = new Set(slipTargets.keys());

    const alloc = computeMainAxisAllocation(node.children, nodeIsRow, nodePixels, getNodeSize, gapPx, slipChildIds);
    const pixelToSizeRatio = alloc.pixelToSizeRatio;

    // Phase A — base rect for every child. A slip child gets zero main-axis
    // width here (via `alloc.px[i]` above) — a placeholder Phase B replaces
    // with its docked chip rect. A minimized child renders as a chip: in a
    // Row parent its cross-axis (height) is clamped to its stacked-chip
    // total — one header height per leaf, via the SAME minimizedFixedPx
    // formula the main-axis allocation uses, not just HeaderHeightPx. A
    // fully-minimized BRANCH holding N leaves needs N header-heights of
    // cross-axis space: without this, the branch's own rect stays
    // full-height while its recursive Column layout only fills the top
    // N*(header+gap) px of it, leaving dead space below the chip stack.
    let lastChildRect: Dimensions;
    node.children.forEach((child, i) => {
        const minimizedChild = isEffectivelyMinimized(child);
        const rect: Dimensions = {
            top: !nodeIsRow && lastChildRect ? lastChildRect.top + lastChildRect.height : nodeRect.top,
            left: nodeIsRow && lastChildRect ? lastChildRect.left + lastChildRect.width : nodeRect.left,
            width: nodeIsRow ? alloc.px[i] : nodeRect.width,
            height: nodeIsRow
                ? minimizedChild
                    ? Math.min(minimizedCrossAxisPx(child, gapPx), nodeRect.height)
                    : nodeRect.height
                : alloc.px[i],
        };
        additionalPropsMap[child.id] = {
            rect,
            transform: setTransform(rect),
            treeKey: additionalProps.treeKey + i,
        };
        lastChildRect = rect;
    });

    // Phase B (Row only) — dock each slip child's header chip(s) onto its
    // target: shrink the target's rect and push it down to make room above
    // it, then stack the slip children's own chips in that reserved space
    // in row order. Grouped by target so multiple simultaneous slips onto
    // the same anchor stack predictably. Mutates additionalPropsMap entries
    // Phase A already wrote — this is why resize-handle generation (Phase C)
    // reads rects back from the map instead of a loop-local variable.
    if (nodeIsRow && slipChildIds.size > 0) {
        const slipsByTargetId = new Map<string, LayoutNode[]>();
        for (const child of node.children) {
            const target = slipTargets.get(child.id);
            if (!target) continue;
            const list = slipsByTargetId.get(target.id) ?? [];
            list.push(child);
            slipsByTargetId.set(target.id, list);
        }
        for (const [targetId, slipChildren] of slipsByTargetId) {
            const targetProps = additionalPropsMap[targetId];
            if (!targetProps?.rect) continue;
            const originalTop = targetProps.rect.top;
            const totalSlipHeight = slipChildren.reduce((s, c) => s + minimizedCrossAxisPx(c, gapPx), 0);
            const clampedSlipHeight = Math.min(totalSlipHeight, targetProps.rect.height);
            // Scale EACH chip down proportionally when the group's combined
            // height exceeds the target's available space (several minimized
            // panes converging on one small anchor) — mirrors
            // computeMainAxisAllocation's `scale` factor for its analogous
            // fixed-chip path. Without this, chips were sized at their raw
            // unclamped height and the stack overflowed past
            // originalTop + clampedSlipHeight, outside the row's bounds
            // [reagent P1].
            const scale = totalSlipHeight > clampedSlipHeight && totalSlipHeight > 0 ? clampedSlipHeight / totalSlipHeight : 1;
            const shrunkRect: Dimensions = {
                ...targetProps.rect,
                top: originalTop + clampedSlipHeight,
                height: targetProps.rect.height - clampedSlipHeight,
            };
            additionalPropsMap[targetId] = {
                ...targetProps,
                rect: shrunkRect,
                transform: setTransform(shrunkRect),
            };
            let chipTop = originalTop;
            for (const slipChild of slipChildren) {
                const chipHeight = minimizedCrossAxisPx(slipChild, gapPx) * scale;
                const chipRect: Dimensions = {
                    top: chipTop,
                    left: shrunkRect.left,
                    width: shrunkRect.width,
                    height: chipHeight,
                };
                additionalPropsMap[slipChild.id] = {
                    ...additionalPropsMap[slipChild.id],
                    rect: chipRect,
                    transform: setTransform(chipRect),
                };
                chipTop += chipHeight;
            }
        }
    }

    // Phase C — resize handles between consecutive REAL (non-minimized,
    // non-slip) slots only; a minimized/slip edge gets no handle at all —
    // chip geometry is derived, there is nothing to resize. Reads FINAL
    // rects from additionalPropsMap (not a loop-local value) since Phase B
    // may have adjusted a target's rect after Phase A first computed it.
    // parentIndex is the child-array index (i - 1), not resizeHandles.length
    // — onResizeMove uses it to look up flanking children by that index, and
    // Phase B never removes entries from node.children, only adjusts rects.
    const resizeHandles: ResizeHandleProps[] = [];
    for (let i = 1; i < node.children.length; i++) {
        const prevChild = node.children[i - 1];
        const child = node.children[i];
        if (isEffectivelyMinimized(prevChild) || isEffectivelyMinimized(child)) continue;
        const prevRect = additionalPropsMap[prevChild.id].rect;
        const halfResizeHandleSizePx = resizeHandleSizePx / 2;
        const resizeHandleDimensions: Dimensions = {
            top: nodeIsRow ? prevRect.top : prevRect.top + prevRect.height - halfResizeHandleSizePx,
            left: nodeIsRow ? prevRect.left + prevRect.width - halfResizeHandleSizePx : prevRect.left,
            width: nodeIsRow ? resizeHandleSizePx : prevRect.width,
            height: nodeIsRow ? prevRect.height : resizeHandleSizePx,
        };
        resizeHandles.push({
            id: `${node.id}-${i - 1}`,
            parentNodeId: node.id,
            parentIndex: i - 1,
            transform: setTransform(resizeHandleDimensions, true, false),
            flexDirection: node.flexDirection,
            centerPx: (nodeIsRow ? resizeHandleDimensions.left : resizeHandleDimensions.top) + halfResizeHandleSizePx,
            perpMinPx: nodeIsRow ? resizeHandleDimensions.top : resizeHandleDimensions.left,
            perpMaxPx: nodeIsRow
                ? resizeHandleDimensions.top + resizeHandleDimensions.height
                : resizeHandleDimensions.left + resizeHandleDimensions.width,
        });
    }

    additionalPropsMap[node.id] = {
        ...additionalProps,
        ...(node.data?.blockId ? { rect: nodeRect } : {}),
        pixelToSizeRatio,
        resizeHandles,
    };
}

/**
 * Gets normalized dimensions for the TileLayout container.
 * @param model The LayoutModel instance.
 * @returns The normalized dimensions for the TileLayout container.
 */
export function getBoundingRect(model: LayoutModel): Dimensions {
    const boundingRect = model.displayContainerRef.current.getBoundingClientRect();
    return { top: 0, left: 0, width: boundingRect.width, height: boundingRect.height };
}

/**
 * Compute a clockwise spiral ordering of leaf nodes based on their screen positions.
 * Peels the outer ring clockwise (top row L→R, right col T→B, bottom row R→L,
 * left col B→T), then recurses into the remaining interior panes.
 *
 * Tab = spiral inward (forward through this order).
 * Ctrl+Tab = spiral outward (backward through this order).
 *
 * Example for a 5-column, 3-row grid (15 panes):
 *   ┌───┬───┬───┬───┬───┐
 *   │ 1 │ 2 │ 3 │ 4 │ 5 │   top row L→R
 *   ├───┼───┼───┼───┼───┤
 *   │12 │13 │14 │15 │ 6 │   right col T→B (6), left col B→T (12), inner (13-15)
 *   ├───┼───┼───┼───┼───┤
 *   │11 │10 │ 9 │ 8 │ 7 │   bottom row R→L
 *   └───┴───┴───┴───┴───┘
 *   Outer ring: 1→2→3→4→5→6→7→8→9→10→11→12, Inner: 13→14→15
 */
export function computeSpiralOrder(
    leafOrder: LeafOrderEntry[],
    additionalProps: Record<string, LayoutNodeAdditionalProps>
): LeafOrderEntry[] {
    if (leafOrder.length <= 1) return [...leafOrder];

    type EntryWithRect = LeafOrderEntry & { rect: Dimensions };
    const entries: EntryWithRect[] = leafOrder
        .map((entry) => ({
            ...entry,
            rect: additionalProps[entry.nodeid]?.rect,
        }))
        .filter((e): e is EntryWithRect => e.rect != null);

    if (entries.length <= 1) return entries.map(({ nodeid, blockid }) => ({ nodeid, blockid }));

    const result: LeafOrderEntry[] = [];
    const remaining = [...entries];
    const epsilon = 2;

    while (remaining.length > 0) {
        if (remaining.length === 1) {
            result.push({ nodeid: remaining[0].nodeid, blockid: remaining[0].blockid });
            break;
        }

        const minLeft = Math.min(...remaining.map((e) => e.rect.left));
        const maxRight = Math.max(...remaining.map((e) => e.rect.left + e.rect.width));
        const minTop = Math.min(...remaining.map((e) => e.rect.top));
        const maxBottom = Math.max(...remaining.map((e) => e.rect.top + e.rect.height));

        // Classify panes by which edge(s) of the bounding box they touch
        const onTop = remaining.filter((e) => e.rect.top <= minTop + epsilon);
        const onRight = remaining.filter((e) => e.rect.left + e.rect.width >= maxRight - epsilon);
        const onBottom = remaining.filter((e) => e.rect.top + e.rect.height >= maxBottom - epsilon);
        const onLeft = remaining.filter((e) => e.rect.left <= minLeft + epsilon);

        // Build the outer ring in clockwise order, deduplicating
        const seen = new Set<string>();
        const ring: EntryWithRect[] = [];
        const addToRing = (entries: EntryWithRect[]) => {
            for (const e of entries) {
                if (!seen.has(e.nodeid)) {
                    seen.add(e.nodeid);
                    ring.push(e);
                }
            }
        };

        // Top edge: left to right
        onTop.sort((a, b) => a.rect.left - b.rect.left);
        addToRing(onTop);

        // Right edge: top to bottom (skip corner already added)
        onRight.sort((a, b) => a.rect.top - b.rect.top);
        addToRing(onRight);

        // Bottom edge: right to left (skip corner already added)
        onBottom.sort((a, b) => b.rect.left - a.rect.left);
        addToRing(onBottom);

        // Left edge: bottom to top (skip corners already added)
        onLeft.sort((a, b) => b.rect.top - a.rect.top);
        addToRing(onLeft);

        if (ring.length === 0) {
            // Shouldn't happen, but safety: dump everything and stop
            result.push(...remaining.map(({ nodeid, blockid }) => ({ nodeid, blockid })));
            break;
        }

        result.push(...ring.map(({ nodeid, blockid }) => ({ nodeid, blockid })));

        if (ring.length === remaining.length) {
            // All panes were in the outer ring — we're done
            break;
        }

        // Remove outer ring, continue with interior panes
        remaining.splice(0, remaining.length, ...remaining.filter((e) => !seen.has(e.nodeid)));
    }

    return result;
}

/**
 * Compute sorted leaf order from leaf nodes and their additional properties.
 * @param leafs The leaf nodes.
 * @param additionalProps The additional properties for all nodes.
 * @returns Sorted leaf order entries.
 */
function getLeafOrder(
    leafs: LayoutNode[],
    additionalProps: Record<string, LayoutNodeAdditionalProps>
): LeafOrderEntry[] {
    return leafs
        .map((node) => ({ nodeid: node.id, blockid: node.data.blockId }) as LeafOrderEntry)
        .sort((a, b) => {
            const treeKeyA = additionalProps[a.nodeid]?.treeKey;
            const treeKeyB = additionalProps[b.nodeid]?.treeKey;
            if (!treeKeyA || !treeKeyB) return;
            return treeKeyA.localeCompare(treeKeyB);
        });
}
