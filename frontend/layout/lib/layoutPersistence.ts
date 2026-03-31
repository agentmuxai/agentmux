// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { batch } from "solid-js";
import { fireAndForget } from "@/util/util";
import { findNodeByBlockId, newLayoutNode } from "./layoutNode";
import {
    LayoutTreeActionType,
    LayoutTreeClearTreeAction,
    LayoutTreeDeleteNodeAction,
    LayoutTreeInsertNodeAction,
    LayoutTreeInsertNodeAtIndexAction,
    LayoutTreeReplaceNodeAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
    LayoutTreeState,
} from "./types";
import type { LayoutModel } from "./layoutModel";

/**
 * Initialize the layout tree from the persisted WaveObject state.
 * @param model The LayoutModel instance.
 */
export function initializeFromWaveObject(model: LayoutModel) {
    const waveObjState = model.getter(model.waveObjectAtom);

    const initialState: LayoutTreeState = {
        rootNode: waveObjState?.rootnode,
        focusedNodeId: waveObjState?.focusednodeid,
        magnifiedNodeId: waveObjState?.magnifiednodeid,
        leafOrder: undefined,
        pendingBackendActions: waveObjState?.pendingbackendactions,
    };

    model.treeState = initialState;
    model.magnifiedNodeId = initialState.magnifiedNodeId;
    model.setter(model.localTreeStateAtom, { ...initialState });

    if (initialState.pendingBackendActions?.length) {
        fireAndForget(() => processPendingBackendActions(model));
    } else {
        model.updateTree();
    }
}

/**
 * Handle a WaveObject update notification from the backend.
 * @param model The LayoutModel instance.
 */
export function onBackendUpdate(model: LayoutModel) {
    const waveObj = model.getter(model.waveObjectAtom);
    if (!waveObj) return;

    // If the model has no rootNode but the backend does, re-initialize.
    // This handles tear-off windows where the LayoutState wasn't loaded
    // when the LayoutModel was first constructed.
    if (!model.treeState.rootNode && waveObj.rootnode) {
        initializeFromWaveObject(model);
        return;
    }

    const pendingActions = waveObj?.pendingbackendactions;
    if (pendingActions?.length) {
        fireAndForget(() => processPendingBackendActions(model));
    }
}

/**
 * Process all pending backend actions from the WaveObject queue.
 * @param model The LayoutModel instance.
 */
export async function processPendingBackendActions(model: LayoutModel) {
    const waveObj = model.getter(model.waveObjectAtom);
    const actions = waveObj?.pendingbackendactions;
    if (!actions?.length) return;

    model.treeState.pendingBackendActions = undefined;

    for (const action of actions) {
        if (!action.actionid) {
            console.warn("Dropping layout action without actionid:", action);
            continue;
        }
        if (model.processedActionIds.has(action.actionid)) {
            continue;
        }
        model.processedActionIds.add(action.actionid);
        await handleBackendAction(model, action);
    }

    batch(() => {
        model.updateTree();
        model.setter(model.localTreeStateAtom, { ...model.treeState });
    });
    model.persistToBackend();
}

/**
 * Handle a single backend layout action.
 * @param model The LayoutModel instance.
 * @param action The layout action data from the backend.
 */
async function handleBackendAction(model: LayoutModel, action: LayoutActionData) {
    switch (action.actiontype) {
        case LayoutTreeActionType.InsertNode: {
            if (action.ephemeral) {
                model.newEphemeralNode(action.blockid);
                break;
            }
            const insertNodeAction: LayoutTreeInsertNodeAction = {
                type: LayoutTreeActionType.InsertNode,
                node: newLayoutNode(undefined, undefined, undefined, {
                    blockId: action.blockid,
                }),
                magnified: action.magnified,
                focused: action.focused,
            };
            model.treeReducer(insertNodeAction, false);
            break;
        }
        case LayoutTreeActionType.DeleteNode: {
            let leaf = model?.getNodeByBlockId(action.blockid);

            // If not found in leafs array, search the tree directly (handles orphaned blocks)
            if (!leaf && model.treeState.rootNode) {
                leaf = findNodeByBlockId(model.treeState.rootNode, action.blockid);
                if (leaf) {
                    // Delete directly from tree instead of closeNode (which may expect block to exist)
                    model.treeReducer(
                        {
                            type: LayoutTreeActionType.DeleteNode,
                            nodeId: leaf.id,
                        } as LayoutTreeDeleteNodeAction,
                        false
                    );
                    break;
                }
            }

            if (leaf) {
                await model.closeNode(leaf.id);
            } else {
                console.error(
                    "Cannot apply eventbus layout action DeleteNode, could not find leaf node with blockId",
                    action.blockid
                );
            }
            break;
        }
        case LayoutTreeActionType.InsertNodeAtIndex: {
            if (!action.indexarr) {
                console.error("Cannot apply eventbus layout action InsertNodeAtIndex, indexarr field is missing.");
                break;
            }
            const insertAction: LayoutTreeInsertNodeAtIndexAction = {
                type: LayoutTreeActionType.InsertNodeAtIndex,
                node: newLayoutNode(undefined, action.nodesize, undefined, {
                    blockId: action.blockid,
                }),
                indexArr: action.indexarr,
                magnified: action.magnified,
                focused: action.focused,
            };
            model.treeReducer(insertAction, false);
            break;
        }
        case LayoutTreeActionType.ClearTree: {
            model.treeReducer(
                {
                    type: LayoutTreeActionType.ClearTree,
                } as LayoutTreeClearTreeAction,
                false
            );
            break;
        }
        case LayoutTreeActionType.ReplaceNode: {
            const targetNode = model?.getNodeByBlockId(action.targetblockid);
            if (!targetNode) {
                console.error(
                    "Cannot apply eventbus layout action ReplaceNode, could not find target node with blockId",
                    action.targetblockid
                );
                break;
            }
            const replaceAction: LayoutTreeReplaceNodeAction = {
                type: LayoutTreeActionType.ReplaceNode,
                targetNodeId: targetNode.id,
                newNode: newLayoutNode(undefined, action.nodesize, undefined, {
                    blockId: action.blockid,
                }),
            };
            model.treeReducer(replaceAction, false);
            break;
        }
        case LayoutTreeActionType.SplitHorizontal: {
            const targetNode = model?.getNodeByBlockId(action.targetblockid);
            if (!targetNode) {
                console.error(
                    "Cannot apply eventbus layout action SplitHorizontal, could not find target node with blockId",
                    action.targetblockid
                );
                break;
            }
            if (action.position != "before" && action.position != "after") {
                console.error(
                    "Cannot apply eventbus layout action SplitHorizontal, invalid position",
                    action.position
                );
                break;
            }
            const newNode = newLayoutNode(undefined, action.nodesize, undefined, {
                blockId: action.blockid,
            });
            const splitAction: LayoutTreeSplitHorizontalAction = {
                type: LayoutTreeActionType.SplitHorizontal,
                targetNodeId: targetNode.id,
                newNode: newNode,
                position: action.position,
            };
            model.treeReducer(splitAction, false);
            break;
        }
        case LayoutTreeActionType.SplitVertical: {
            const targetNode = model?.getNodeByBlockId(action.targetblockid);
            if (!targetNode) {
                console.error(
                    "Cannot apply eventbus layout action SplitVertical, could not find target node with blockId",
                    action.targetblockid
                );
                break;
            }
            if (action.position != "before" && action.position != "after") {
                console.error(
                    "Cannot apply eventbus layout action SplitVertical, invalid position",
                    action.position
                );
                break;
            }
            const newNode = newLayoutNode(undefined, action.nodesize, undefined, {
                blockId: action.blockid,
            });
            const splitAction: LayoutTreeSplitVerticalAction = {
                type: LayoutTreeActionType.SplitVertical,
                targetNodeId: targetNode.id,
                newNode: newNode,
                position: action.position,
            };
            model.treeReducer(splitAction, false);
            break;
        }
        default:
            console.warn("unsupported layout action", action);
            break;
    }
}

/**
 * Persist current tree state to the backend WaveObject (debounced).
 * @param model The LayoutModel instance.
 */
export function persistToBackend(model: LayoutModel) {
    if (model.persistDebounceTimer) {
        clearTimeout(model.persistDebounceTimer);
    }

    model.persistDebounceTimer = setTimeout(() => {
        const waveObj = model.getter(model.waveObjectAtom);
        if (!waveObj) return;

        waveObj.rootnode = model.treeState.rootNode;
        waveObj.focusednodeid = model.treeState.focusedNodeId;
        waveObj.magnifiednodeid = model.treeState.magnifiedNodeId;
        waveObj.leaforder = model.treeState.leafOrder;
        waveObj.pendingbackendactions = model.treeState.pendingBackendActions;

        model.setter(model.waveObjectAtom, waveObj);
        model.persistDebounceTimer = null;
    }, 100);
}
