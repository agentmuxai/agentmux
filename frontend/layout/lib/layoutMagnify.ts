// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/app/store/global";
import { findNode } from "./layoutNode";
import { newLayoutNode } from "./layoutNode";
import {
    LayoutNode,
    LayoutNodeAdditionalProps,
    LayoutTreeActionType,
    LayoutTreeDeleteNodeAction,
    LayoutTreeInsertNodeAction,
    LayoutTreeMagnifyNodeToggleAction,
} from "./types";
import { setTransform } from "./utils";
import type { LayoutModel } from "./layoutModel";

// Debug logging function - uses getApi().sendLog() which writes to task dev output
function debugLog(message: string, data?: unknown): void {
    const logLine = `[LAYOUT] ${message}${data !== undefined ? ": " + JSON.stringify(data) : ""}`;
    getApi().sendLog(logLine);
}

/**
 * Toggle magnification of a given node.
 * @param model The LayoutModel instance.
 * @param nodeId The id of the node that is being magnified.
 * @param setState Whether to persist the state change.
 */
export function magnifyNodeToggle(model: LayoutModel, nodeId: string, setState = true) {
    const action: LayoutTreeMagnifyNodeToggleAction = {
        type: LayoutTreeActionType.MagnifyNodeToggle,
        nodeId: nodeId,
    };

    // Unset the last ephemeral node id to ensure the magnify animation sits on top of the layout.
    model.lastEphemeralNodeId = undefined;

    model.treeReducer(action, setState);
}

/**
 * Close a given node and update the tree state.
 * @param model The LayoutModel instance.
 * @param nodeId The id of the node that is being closed.
 */
export async function closeNode(model: LayoutModel, nodeId: string) {
    // DEBUG: Log entry to closeNode
    debugLog("closeNode ENTER", {
        nodeId,
        rootNodeId: model.treeState.rootNode?.id,
        rootNodeChildren: model.treeState.rootNode?.children?.length ?? 0,
        isRootMatch: nodeId === model.treeState.rootNode?.id,
    });

    // DEBUG: Log full tree structure
    const treeSnapshot = getTreeSnapshot(model.treeState.rootNode);
    debugLog("closeNode tree structure", treeSnapshot);

    const nodeToDelete = findNode(model.treeState.rootNode, nodeId);
    if (!nodeToDelete) {
        // TODO: clean up the ephemeral node handling
        // The ephemeral node is not in the tree, so we need to handle it separately.
        const ephemeralNode = model.getter(model.ephemeralNode);
        if (ephemeralNode?.id === nodeId) {
            debugLog("closeNode ephemeral node path");
            model.setter(model.ephemeralNode, undefined);
            model.treeState.focusedNodeId = undefined;
            model.updateTree(false);
            model.setter(model.localTreeStateAtom, { ...model.treeState });
            model.persistToBackend();
            await model.onNodeDelete?.(ephemeralNode.data);
            return;
        }
        debugLog("closeNode ERROR: node not found in tree", nodeId);
        console.error("unable to close node, cannot find it in tree", nodeId);
        return;
    }

    debugLog("closeNode found nodeToDelete", {
        nodeId: nodeToDelete.id,
        nodeData: nodeToDelete.data,
        hasChildren: nodeToDelete.children?.length > 0,
    });

    if (nodeId === model.magnifiedNodeId) {
        magnifyNodeToggle(model, nodeId);
    }
    const deleteAction: LayoutTreeDeleteNodeAction = {
        type: LayoutTreeActionType.DeleteNode,
        nodeId: nodeId,
    };

    debugLog("closeNode calling treeReducer with deleteAction", deleteAction);
    model.treeReducer(deleteAction);

    // DEBUG: Log tree state after deletion
    debugLog("closeNode AFTER treeReducer", {
        rootNodeId: model.treeState.rootNode?.id,
        rootNodeExists: !!model.treeState.rootNode,
        rootNodeChildren: model.treeState.rootNode?.children?.length ?? 0,
    });

    await model.onNodeDelete?.(nodeToDelete.data);
    debugLog("closeNode EXIT");
}

/**
 * Helper to snapshot tree structure for debugging.
 */
function getTreeSnapshot(node: LayoutNode | undefined, depth = 0): unknown {
    if (!node) return null;
    return {
        id: node.id,
        data: node.data,
        childCount: node.children?.length ?? 0,
        children: node.children?.map((c) => getTreeSnapshot(c, depth + 1)) ?? [],
    };
}

/**
 * Shorthand function for closing the focused node in a layout.
 * @param model The LayoutModel instance.
 */
export async function closeFocusedNode(model: LayoutModel) {
    await closeNode(model, model.focusedNodeId);
}

/**
 * Create a new ephemeral (floating) node.
 * @param model The LayoutModel instance.
 * @param blockId The block ID for the new ephemeral node.
 */
export function newEphemeralNode(model: LayoutModel, blockId: string) {
    if (model.getter(model.ephemeralNode)) {
        closeNode(model, model.getter(model.ephemeralNode).id);
    }

    const ephemeralNode = newLayoutNode(undefined, undefined, undefined, { blockId });
    model.setter(model.ephemeralNode, ephemeralNode);

    const addlProps = model.getter(model.additionalProps);
    const leafs = model.getter(model.leafs);
    const boundingRect = model.getBoundingRect();
    const magnifiedNodeSizePct = model.getter(model.magnifiedNodeSizeAtom);
    updateEphemeralNodeProps(model, ephemeralNode, addlProps, leafs, magnifiedNodeSizePct, boundingRect);
    model.setter(model.additionalProps, addlProps);
    model.focusNode(ephemeralNode.id);
}

/**
 * Commit the ephemeral node into the tree as a regular node.
 * @param model The LayoutModel instance.
 */
export function addEphemeralNodeToLayout(model: LayoutModel) {
    const ephemeralNode = model.getter(model.ephemeralNode);
    model.setter(model.ephemeralNode, undefined);
    if (model.magnifiedNodeId) {
        magnifyNodeToggle(model, model.magnifiedNodeId, false);
    }
    model.lastEphemeralNodeId = ephemeralNode.id;
    if (ephemeralNode) {
        const action: LayoutTreeInsertNodeAction = {
            type: LayoutTreeActionType.InsertNode,
            node: ephemeralNode,
            magnified: false,
            focused: false,
        };
        model.treeReducer(action);
    }
}

/**
 * Compute ephemeral node geometry properties.
 * @param model The LayoutModel instance.
 * @param node The ephemeral node.
 * @param addlPropsMap The additional properties map to update.
 * @param leafs The leafs array to append to.
 * @param magnifiedNodeSizePct The magnified node size percentage.
 * @param boundingRect The bounding rect of the layout container.
 */
export function updateEphemeralNodeProps(
    model: LayoutModel,
    node: LayoutNode,
    addlPropsMap: Record<string, LayoutNodeAdditionalProps>,
    leafs: LayoutNode[],
    magnifiedNodeSizePct: number,
    boundingRect: Dimensions
) {
    const ephemeralNodeSizePct = model.magnifiedNodeId
        ? magnifiedNodeSizePct * magnifiedNodeSizePct
        : magnifiedNodeSizePct;
    const ephemeralNodeMarginPct = (1 - ephemeralNodeSizePct) / 2;
    const transform = setTransform(
        {
            top: boundingRect.height * ephemeralNodeMarginPct,
            left: boundingRect.width * ephemeralNodeMarginPct,
            width: boundingRect.width * ephemeralNodeSizePct,
            height: boundingRect.height * ephemeralNodeSizePct,
        },
        true,
        true,
        "var(--zindex-layout-ephemeral-node)"
    );
    addlPropsMap[node.id] = { treeKey: "-1", transform };
    leafs.push(node);
}

/**
 * When a layout is modified and only one leaf is remaining, ensure it is no longer magnified.
 * @param model The LayoutModel instance.
 * @param leafOrder The new leaf order array.
 * @param addlProps The new additional properties object for all leafs.
 */
export function validateMagnifiedNode(
    model: LayoutModel,
    leafOrder: LeafOrderEntry[],
    addlProps: Record<string, LayoutNodeAdditionalProps>
) {
    if (leafOrder.length == 1) {
        const lastLeafId = leafOrder[0].nodeid;
        model.treeState.magnifiedNodeId = undefined;
        model.magnifiedNodeId = undefined;

        // Unset the transform for the sole leaf.
        if (addlProps.hasOwnProperty(lastLeafId)) addlProps[lastLeafId].transform = undefined;
    }
}
