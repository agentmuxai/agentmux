// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { findNode } from "./layoutNode";
import { LayoutTreeActionType, LayoutTreeFocusNodeAction, NavigateDirection, NavigationResult } from "./types";
import { getCenter, navigateDirectionToOffset } from "./utils";
import type { LayoutModel } from "./layoutModel";

/**
 * Checks whether the focused node id has changed and, if so, whether to update the focused node stack.
 * If the focused node was deleted, will pop the latest value from the stack.
 * @param model The LayoutModel instance.
 * @param leafOrder The new leaf order array to use when searching for stale nodes in the stack.
 */
export function validateFocusedNode(model: LayoutModel, leafOrder: LeafOrderEntry[]) {
    if (model.treeState.focusedNodeId !== model.focusedNodeId) {
        console.log("[focus:validate] mismatch treeState.focusedNodeId=", model.treeState.focusedNodeId, "model.focusedNodeId=", model.focusedNodeId, "stack=", [...model.focusedNodeIdStack], "leafOrder=", leafOrder.map(l => l.nodeid));
        // Remove duplicates and stale entries from focus stack.
        const newFocusedNodeIdStack: string[] = [];
        for (const id of model.focusedNodeIdStack) {
            if (leafOrder.find((leafEntry) => leafEntry?.nodeid === id) && !newFocusedNodeIdStack.includes(id))
                newFocusedNodeIdStack.push(id);
        }
        model.focusedNodeIdStack = newFocusedNodeIdStack;
        console.log("[focus:validate] cleaned stack=", [...newFocusedNodeIdStack]);

        // Update the focused node and stack based on the changes in the tree state.
        if (!model.treeState.focusedNodeId) {
            if (model.focusedNodeIdStack.length > 0) {
                model.treeState.focusedNodeId = model.focusedNodeIdStack.shift();
                console.log("[focus:validate] restored from stack:", model.treeState.focusedNodeId);
            } else if (leafOrder.length > 0) {
                // If no nodes are in the stack, use the top left node in the layout.
                model.treeState.focusedNodeId = leafOrder[0].nodeid;
                console.log("[focus:validate] fallback to first leaf:", model.treeState.focusedNodeId);
            }
        }
        model.focusedNodeIdStack.unshift(model.treeState.focusedNodeId);
        console.log("[focus:validate] final focusedNodeId=", model.treeState.focusedNodeId, "stack=", [...model.focusedNodeIdStack]);
    }
}

/**
 * Switch focus to the next node in the given direction in the layout.
 * @param model The LayoutModel instance.
 * @param direction The direction in which to switch focus.
 */
export function switchNodeFocusInDirection(
    model: LayoutModel,
    direction: NavigateDirection
): NavigationResult {
    const curNodeId = model.focusedNodeId;

    // If no node is focused, set focus to the first leaf.
    if (!curNodeId) {
        focusNode(model, model.getter(model.leafOrder)[0].nodeid);
        return { success: true };
    }

    const offset = navigateDirectionToOffset(direction);
    const nodePositions: Map<string, Dimensions> = new Map();
    const leafs = model.getter(model.leafs);
    const addlProps = model.getter(model.additionalProps);
    for (const leaf of leafs) {
        const pos = addlProps[leaf.id]?.rect;
        if (pos) {
            nodePositions.set(leaf.id, pos);
        }
    }
    const curNodePos = nodePositions.get(curNodeId);
    if (!curNodePos) {
        return { success: false };
    }
    nodePositions.delete(curNodeId);
    const boundingRect = model.displayContainerRef?.current.getBoundingClientRect();
    if (!boundingRect) {
        return { success: false };
    }
    const maxX = boundingRect.left + boundingRect.width;
    const maxY = boundingRect.top + boundingRect.height;
    const moveAmount = 10;
    const curPoint = getCenter(curNodePos);

    function findNodeAtPoint(m: Map<string, Dimensions>, p: Point): string {
        for (const [blockId, dimension] of m.entries()) {
            if (
                p.x >= dimension.left &&
                p.x <= dimension.left + dimension.width &&
                p.y >= dimension.top &&
                p.y <= dimension.top + dimension.height
            ) {
                return blockId;
            }
        }
        return null;
    }

    while (true) {
        curPoint.x += offset.x * moveAmount;
        curPoint.y += offset.y * moveAmount;
        if (curPoint.x < 0 || curPoint.x > maxX || curPoint.y < 0 || curPoint.y > maxY) {
            // Determine which boundary was hit
            const result: NavigationResult = { success: false };
            if (curPoint.x < 0) {
                result.atLeft = true;
            }
            if (curPoint.x > maxX) {
                result.atRight = true;
            }
            if (curPoint.y < 0) {
                result.atTop = true;
            }
            if (curPoint.y > maxY) {
                result.atBottom = true;
            }
            return result;
        }
        const nodeId = findNodeAtPoint(nodePositions, curPoint);
        if (nodeId != null) {
            focusNode(model, nodeId);
            return { success: true };
        }
    }
}

/**
 * Switch focus to a node using the given BlockNum.
 * @param model The LayoutModel instance.
 * @param newBlockNum The BlockNum of the node to which focus should switch.
 */
export function switchNodeFocusByBlockNum(model: LayoutModel, newBlockNum: number) {
    const leafOrder = model.getter(model.leafOrder);
    const newLeafIdx = newBlockNum - 1;
    if (newLeafIdx < 0 || newLeafIdx >= leafOrder.length) {
        return;
    }
    const leaf = leafOrder[newLeafIdx];
    focusNode(model, leaf.nodeid);
}

/**
 * Set the layout to focus on the given node.
 * @param model The LayoutModel instance.
 * @param nodeId The id of the node that is being focused.
 */
export function focusNode(model: LayoutModel, nodeId: string) {
    if (model.focusedNodeId === nodeId) return;
    console.log("[focus:focusNode] changing focus from", model.focusedNodeId, "to", nodeId);
    let layoutNode = findNode(model.treeState?.rootNode, nodeId);
    if (!layoutNode) {
        const ephemeralNode = model.getter(model.ephemeralNode);
        if (ephemeralNode?.id === nodeId) {
            layoutNode = ephemeralNode;
        } else {
            console.error("[focus:focusNode] unable to focus node, cannot find it in tree", nodeId);
            return;
        }
    }
    const action: LayoutTreeFocusNodeAction = {
        type: LayoutTreeActionType.FocusNode,
        nodeId: nodeId,
    };

    model.treeReducer(action);
}

/**
 * Focus the first node in the layout.
 * @param model The LayoutModel instance.
 */
export function focusFirstNode(model: LayoutModel) {
    const leafOrder = model.getter(model.leafOrder);
    if (leafOrder.length > 0) {
        focusNode(model, leafOrder[0].nodeid);
    }
}

/**
 * Get the block ID of the first leaf node.
 * @param model The LayoutModel instance.
 * @returns The block ID of the first leaf, or undefined if no leafs exist.
 */
export function getFirstBlockId(model: LayoutModel): string | undefined {
    const leafOrder = model.getter(model.leafOrder);
    if (leafOrder.length > 0) {
        return leafOrder[0].blockid;
    }
    return undefined;
}
