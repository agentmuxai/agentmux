// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { batch } from "solid-js";
import { fireAndForget } from "@/util/util";
import { findNodeByBlockId, newLayoutNode, walkNodes } from "./layoutNode";
import { rebuildMinimizedSet } from "./layoutMinimize";
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
    rebuildMinimizedSet(model);

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
    } else {
        pruneDanglingLeaves(model);
    }
}

/**
 * Remove leaves whose block is NOT in the tab's `blockids` — dangling
 * references left behind when a frontend's debounced persist clobbers a
 * queued backend layout action (the stale-tree resurrection issue,
 * INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08).
 * A dangling leaf renders a block owned by ANOTHER tab; since every tab
 * stays mounted, the same block mounts twice and the block-component
 * registry breaks — observed as a fully non-responsive tab.
 *
 * Ownership (`Tab.blockids`) is reducer-owned truth, so pruning against
 * it is always safe: this only ever REMOVES leaves for disowned blocks,
 * never touches leaves whose block is owned (a queued insert for a
 * newly-owned block is unaffected — the leaf simply isn't there yet).
 *
 * Reactivity note: this reads `model.tabAtom()` inside the same effect
 * that drives `onBackendUpdate` (layoutModelHooks), so `Tab` becomes a
 * tracked dependency — a MoveBlock updating `blockids` re-runs the
 * effect and prunes even when the queued layout delete was lost.
 */
export function pruneDanglingLeaves(model: LayoutModel) {
    const rootNode = model.treeState?.rootNode;
    const tab = model.tabAtom?.();
    if (!rootNode || !tab?.blockids) return;
    const owned = new Set(tab.blockids);
    const danglingIds: string[] = [];
    walkNodes(rootNode, (node) => {
        if (!node.children?.length && node.data?.blockId && !owned.has(node.data.blockId)) {
            danglingIds.push(node.id);
        }
    });
    if (!danglingIds.length) return;
    console.warn("[layout] pruning dangling leaves (blocks not owned by this tab):", danglingIds);
    for (const nodeId of danglingIds) {
        model.treeReducer(
            { type: LayoutTreeActionType.DeleteNode, nodeId } as LayoutTreeDeleteNodeAction,
            false,
        );
    }
    batch(() => {
        model.updateTree();
        model.setter(model.localTreeStateAtom, { ...model.treeState });
    });
    model.persistToBackend();
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
    pruneDanglingLeaves(model);
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
                // R1 (#1681): a backend "delete" layout action means "remove this
                // node from the layout tree" — NOT "delete the block". Every
                // backend emitter of this action is a MOVE (tear_off_block,
                // TearOffBlock / RedockFloatingPane / PromoteBlockToTab via
                // queue_source_layout_delete); the block lives on in its new tab.
                // Using closeNode() here ran onNodeDelete → DeleteBlock and
                // destroyed the just-moved block (empty-slot redock, logo-only
                // floater, "block not found"). Remove the node directly, like the
                // orphaned-block branch above. Genuine pane CLOSE deletes the
                // block through the frontend closeNode path, not this action — so
                // it is unaffected. This makes the dedicated block-move guard
                // obsolete (deleted).
                model.treeReducer(
                    {
                        type: LayoutTreeActionType.DeleteNode,
                        nodeId: leaf.id,
                    } as LayoutTreeDeleteNodeAction,
                    false
                );
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
                sizeFraction: action.nodesizefraction,
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
                sizeFraction: action.nodesizefraction,
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
