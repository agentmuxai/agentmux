// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { batch } from "solid-js";
import { balanceNode, walkNodes } from "./layoutNode";
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
        if (balanceTree) model.treeState.rootNode = balanceNode(model.treeState.rootNode, callback);
        else walkNodes(model.treeState.rootNode, callback);

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
        batch(() => {
            model.setter(model.leafs, sortedLeafs);
            model.setter(model.leafOrder, model.treeState.leafOrder);
            model.setter(model.additionalProps, newAdditionalProps);
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
            if (model.magnifiedNodeId === node.id) {
                const magnifiedNodeMarginPct = (1 - magnifiedNodeSizePct) / 2;
                const transform = setTransform(
                    {
                        top: boundingRect.height * magnifiedNodeMarginPct,
                        left: boundingRect.width * magnifiedNodeMarginPct,
                        width: boundingRect.width * magnifiedNodeSizePct,
                        height: boundingRect.height * magnifiedNodeSizePct,
                    },
                    true,
                    true,
                    "var(--zindex-layout-magnified-node)"
                );
                addlProps.transform = transform;
            }
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
    const totalChildrenSize = node.children.reduce((acc, child) => acc + getNodeSize(child), 0);
    const pixelToSizeRatio = totalChildrenSize / nodePixels;

    let lastChildRect: Dimensions;
    const resizeHandles: ResizeHandleProps[] = [];

    node.children.forEach((child, i) => {
        const childSize = getNodeSize(child);
        const rect: Dimensions = {
            top: !nodeIsRow && lastChildRect ? lastChildRect.top + lastChildRect.height : nodeRect.top,
            left: nodeIsRow && lastChildRect ? lastChildRect.left + lastChildRect.width : nodeRect.left,
            width: nodeIsRow ? childSize / pixelToSizeRatio : nodeRect.width,
            height: nodeIsRow ? nodeRect.height : childSize / pixelToSizeRatio,
        };
        const transform = setTransform(rect);
        additionalPropsMap[child.id] = {
            rect,
            transform,
            treeKey: additionalProps.treeKey + i,
        };

        // We only want the resize handles in between nodes, this ensures we have n-1 handles.
        if (lastChildRect) {
            const resizeHandleIndex = resizeHandles.length;
            const halfResizeHandleSizePx = resizeHandleSizePx / 2;
            const resizeHandleDimensions: Dimensions = {
                top: nodeIsRow
                    ? lastChildRect.top
                    : lastChildRect.top + lastChildRect.height - halfResizeHandleSizePx,
                left: nodeIsRow
                    ? lastChildRect.left + lastChildRect.width - halfResizeHandleSizePx
                    : lastChildRect.left,
                width: nodeIsRow ? resizeHandleSizePx : lastChildRect.width,
                height: nodeIsRow ? lastChildRect.height : resizeHandleSizePx,
            };
            resizeHandles.push({
                id: `${node.id}-${resizeHandleIndex}`,
                parentNodeId: node.id,
                parentIndex: resizeHandleIndex,
                transform: setTransform(resizeHandleDimensions, true, false),
                flexDirection: node.flexDirection,
                centerPx:
                    (nodeIsRow ? resizeHandleDimensions.left : resizeHandleDimensions.top) + halfResizeHandleSizePx,
            });
        }
        lastChildRect = rect;
    });

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
export function getLeafOrder(
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
