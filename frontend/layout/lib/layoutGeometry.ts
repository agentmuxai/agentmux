// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/app/store/global";
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

// Debug logging function - uses getApi().sendLog() which writes to task dev output
function debugLog(message: string, data?: unknown): void {
    const logLine = `[LAYOUT] ${message}${data !== undefined ? ": " + JSON.stringify(data) : ""}`;
    getApi().sendLog(logLine);
}

/**
 * Recursively walks the tree to find leaf nodes, update the resize handles, and compute additional properties for each node.
 * @param model The LayoutModel instance.
 * @param balanceTree Whether the tree should also be balanced as it is walked. Defaults to true.
 */
export function updateTree(model: LayoutModel, balanceTree = true) {
    debugLog("updateTree ENTER", {
        balanceTree,
        hasDisplayContainer: !!model.displayContainerRef.current,
        rootNodeId: model.treeState.rootNode?.id,
        rootNodeChildren: model.treeState.rootNode?.children?.length ?? 0,
    });

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

        const magnifiedNodeSize = model.getter(model.magnifiedNodeSizeAtom);

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

        debugLog("updateTree AFTER balanceNode", {
            rootNodeId: model.treeState.rootNode?.id,
            rootNodeExists: !!model.treeState.rootNode,
            rootNodeChildren: model.treeState.rootNode?.children?.length ?? 0,
            newLeafsCount: newLeafs.length,
            newLeafIds: newLeafs.map((l) => l.id),
        });

        // Process ephemeral node, if present.
        const ephemeralNode = model.getter(model.ephemeralNode);
        if (ephemeralNode) {
            console.log("updateTree ephemeralNode", ephemeralNode);
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
        debugLog("updateTree setting leafs", {
            leafCount: sortedLeafs.length,
            leafOrderCount: model.treeState.leafOrder.length,
            leafIds: sortedLeafs.map((l) => l.id),
            additionalPropsKeys: Object.keys(newAdditionalProps),
        });
        // DEBUG: Log before and after setter calls
        debugLog("updateTree SETTER START", {
            currentLeafsCount: model.getter(model.leafs)?.length,
        });
        model.setter(model.leafs, sortedLeafs);
        model.setter(model.leafOrder, model.treeState.leafOrder);
        model.setter(model.additionalProps, newAdditionalProps);
        debugLog("updateTree SETTER DONE", {
            newLeafsCount: model.getter(model.leafs)?.length,
        });
        debugLog("updateTree EXIT - success");
    } else {
        debugLog("updateTree EXIT - no displayContainerRef");
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
            debugLog("updateTreeHelper creating additionalProps for root leaf", {
                nodeId: node.id,
                boundingRect,
            });
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
    debugLog("updateTreeHelper processing parent node", {
        nodeId: node.id,
        isRoot: node.id === model.treeState.rootNode?.id,
        childCount: node.children.length,
        nodeRect,
        nodeIsRow,
    });

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
        debugLog("updateTreeHelper set child additionalProps", {
            childId: child.id,
            childBlockId: child.data?.blockId,
            rect,
            hasTransform: !!transform,
        });

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
 * Processes outermost panes first (clockwise), then recurses inward.
 * For Tab cycling: forward = spiral inward, backward = spiral outward.
 */
export function computeSpiralOrder(
    leafOrder: LeafOrderEntry[],
    additionalProps: Record<string, LayoutNodeAdditionalProps>
): LeafOrderEntry[] {
    if (leafOrder.length <= 2) return [...leafOrder];

    const entries = leafOrder
        .map((entry) => ({
            ...entry,
            rect: additionalProps[entry.nodeid]?.rect,
        }))
        .filter((e) => e.rect);

    if (entries.length <= 2) return entries.map(({ nodeid, blockid }) => ({ nodeid, blockid }));

    const result: LeafOrderEntry[] = [];
    const remaining = [...entries];

    while (remaining.length > 0) {
        if (remaining.length <= 2) {
            result.push(...remaining.map(({ nodeid, blockid }) => ({ nodeid, blockid })));
            break;
        }

        const epsilon = 2;
        const minLeft = Math.min(...remaining.map((e) => e.rect.left));
        const maxRight = Math.max(...remaining.map((e) => e.rect.left + e.rect.width));
        const minTop = Math.min(...remaining.map((e) => e.rect.top));
        const maxBottom = Math.max(...remaining.map((e) => e.rect.top + e.rect.height));

        const outer = remaining.filter(
            (e) =>
                e.rect.left <= minLeft + epsilon ||
                e.rect.left + e.rect.width >= maxRight - epsilon ||
                e.rect.top <= minTop + epsilon ||
                e.rect.top + e.rect.height >= maxBottom - epsilon
        );

        const toSort = outer.length === 0 || outer.length === remaining.length ? remaining : outer;
        const cx = (minLeft + maxRight) / 2;
        const cy = (minTop + maxBottom) / 2;

        toSort.sort((a, b) => {
            const ax = a.rect.left + a.rect.width / 2;
            const ay = a.rect.top + a.rect.height / 2;
            const bx = b.rect.left + b.rect.width / 2;
            const by = b.rect.top + b.rect.height / 2;
            // atan2 gives angle from center; shift so top-left (~ -PI) starts at 0
            const angleA = (Math.atan2(ay - cy, ax - cx) + Math.PI * 2) % (Math.PI * 2);
            const angleB = (Math.atan2(by - cy, bx - cx) + Math.PI * 2) % (Math.PI * 2);
            return angleA - angleB;
        });

        if (outer.length === 0 || outer.length === remaining.length) {
            result.push(...remaining.map(({ nodeid, blockid }) => ({ nodeid, blockid })));
            break;
        }

        result.push(...outer.map(({ nodeid, blockid }) => ({ nodeid, blockid })));
        const outerIds = new Set(outer.map((e) => e.nodeid));
        remaining.splice(0, remaining.length, ...remaining.filter((e) => !outerIds.has(e.nodeid)));
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
