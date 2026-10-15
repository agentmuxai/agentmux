// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { batch } from "solid-js";
import { fireAndForget } from "@/util/util";
import { findNodeByBlockId, newLayoutNode } from "./layoutNode";
import { rebuildMinimizedSet } from "./layoutMinimize";
import { removeBlockFromLeaf, removeLeafEmptiedByMove } from "./layoutMagnify";
import { queueLayoutCommand, reconcileWithStoredLayout } from "./layoutIntents";
import { activateMemberInStack, addMemberToStack, moveMemberAcrossStacks, moveMemberInStack } from "./stackMembers";
import type { PaneTabEditResult } from "@/types/rpc/PaneTabEditResult";
import {
    LayoutTreeActionType,
    LayoutTreeClearTreeAction,
    LayoutTreeInsertNodeAction,
    LayoutTreeInsertNodeAtIndexAction,
    LayoutTreeReplaceNodeAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
    LayoutTreeState,
} from "./types";
import type { LayoutModel } from "./layoutModel";

/**
 * Initialize the layout tree from the persisted MuxObject state.
 * @param model The LayoutModel instance.
 */
export function initializeFromMuxObject(model: LayoutModel) {
    const muxObjState = model.getter(model.muxObjectAtom);

    const initialState: LayoutTreeState = {
        rootNode: muxObjState?.rootnode,
        focusedNodeId: muxObjState?.focusednodeid,
        magnifiedNodeId: muxObjState?.magnifiednodeid,
        leafOrder: undefined,
        pendingBackendActions: muxObjState?.pendingbackendactions,
    };

    model.treeState = initialState;
    model.magnifiedNodeId = initialState.magnifiedNodeId;
    model.setter(model.localTreeStateAtom, { ...initialState });
    rebuildMinimizedSet(model);

    // srv applied every queued action to the tree just loaded: they are
    // seen, not replayed.
    for (const action of initialState.pendingBackendActions ?? []) {
        if (action.actionid) model.processedActionIds.add(action.actionid);
    }
    model.updateTree();
}

/**
 * Handle a MuxObject update notification from the backend.
 * @param model The LayoutModel instance.
 */
export function onBackendUpdate(model: LayoutModel) {
    if (model.disposed) return;
    const muxObj = model.getter(model.muxObjectAtom);
    if (!muxObj) return;

    // If the model has no rootNode but the backend does, re-initialize.
    // This handles tear-off windows where the LayoutState wasn't loaded
    // when the LayoutModel was first constructed.
    if (!model.treeState.rootNode && muxObj.rootnode) {
        initializeFromMuxObject(model);
        return;
    }

    const pendingActions = muxObj?.pendingbackendactions;
    if (pendingActions?.length) {
        fireAndForget(() => processPendingBackendActions(model));
    }
    reconcileWithStoredLayout(model);
}

/**
 * Process all pending backend actions from the MuxObject queue.
 * @param model The LayoutModel instance.
 */
export async function processPendingBackendActions(model: LayoutModel) {
    if (model.disposed) return;
    const muxObj = model.getter(model.muxObjectAtom);
    const actions = muxObj?.pendingbackendactions;
    if (!actions?.length) return;

    model.treeState.pendingBackendActions = undefined;

    model.processingBackendActions = true;
    try {
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
    } finally {
        model.processingBackendActions = false;
    }

    batch(() => {
        model.updateTree();
        model.setter(model.localTreeStateAtom, { ...model.treeState });
    });
    // Nothing to send: srv applied each action before queueing it.
    reconcileWithStoredLayout(model);
}

/** A queued placement srv already applied, found in this tree (a row this
 *  model loaded after it, or a second delivery). */
function alreadyPlaced(model: LayoutModel, action: LayoutActionData): boolean {
    return !!model.treeState.rootNode && !!findNodeByBlockId(model.treeState.rootNode, action.blockid);
}

/** The leaf srv placed: its node id when srv sent one. */
function mirroredLeaf(action: LayoutActionData) {
    const leaf = newLayoutNode(undefined, action.nodesize, undefined, { blockId: action.blockid });
    if (action.nodeid) leaf.id = action.nodeid;
    return leaf;
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
            if (alreadyPlaced(model, action)) break;
            const insertNodeAction: LayoutTreeInsertNodeAction = {
                type: LayoutTreeActionType.InsertNode,
                node: mirroredLeaf(action),
                magnified: action.magnified,
                focused: action.focused,
            };
            model.treeReducer(insertNodeAction, false, { actionId: action.actionid });
            break;
        }
        case LayoutTreeActionType.DeleteNode: {
            let leaf = model?.getNodeByBlockId(action.blockid);

            // If not found in leafs array, search the tree directly (handles
            // orphaned blocks, and a leafs array that is stale mid-batch — it
            // is only refreshed once, after every action in the batch).
            if (!leaf && model.treeState.rootNode) {
                leaf = findNodeByBlockId(model.treeState.rootNode, action.blockid);
            }

            // The action names ONE block, and every backend emitter of it is a
            // MOVE (TearOffBlock / RedockFloatingPane / PromoteBlockToTab via
            // queue_source_layout_delete, or delete_block's layout prune): the
            // block lives on elsewhere. So:
            // - `getNodeByBlockId` also matches a background tab of a stacked
            //   pane; deleting the whole leaf removed every other tab with it
            //   (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §2.2, §3.4).
            //   Only this member leaves; the leaf goes only when it was the last.
            // - R1 (#1681): never closeNode() here. It ran onNodeDelete →
            //   DeleteBlock and destroyed the just-moved block. A genuine pane
            //   close deletes its block through the frontend closeNode path,
            //   not this action.
            // Both rules live in one shared helper, also used by the tear-off
            // paths (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.5). Committed
            // by processPendingBackendActions' updateTree; srv applied it already.
            // No leaf: this window removed it already (a close it made).
            if (leaf) removeBlockFromLeaf(model, leaf, action.blockid);
            break;
        }
        case LayoutTreeActionType.InsertNodeAtIndex: {
            if (alreadyPlaced(model, action)) break;
            if (!action.indexarr) {
                console.error("Cannot apply eventbus layout action InsertNodeAtIndex, indexarr field is missing.");
                break;
            }
            const insertAction: LayoutTreeInsertNodeAtIndexAction = {
                type: LayoutTreeActionType.InsertNodeAtIndex,
                node: mirroredLeaf(action),
                indexArr: action.indexarr,
                magnified: action.magnified,
                focused: action.focused,
            };
            model.treeReducer(insertAction, false, { actionId: action.actionid });
            break;
        }
        case LayoutTreeActionType.StackPush: {
            // The backend already placed the block (`CreateBlockInStack`);
            // mirror it so the tab appears. SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3.
            let leaf = model?.getNodeByBlockId(action.targetblockid);
            if (!leaf && model.treeState.rootNode) {
                leaf = findNodeByBlockId(model.treeState.rootNode, action.targetblockid);
            }
            if (leaf?.data) {
                addMemberToStack(leaf.data, action.blockid, true);
            } else {
                console.error(
                    "Cannot apply layout action StackPush, no pane holds blockId",
                    action.targetblockid
                );
            }
            break;
        }
        case LayoutTreeActionType.StackMove: {
            // Mirrors StackPush's leaf resolution above; `activate` rides on
            // `action.focused` (see `queue_target_stack_move`'s doc comment,
            // crates/srv/src/server/service/layout_helpers.rs).
            let leaf = model?.getNodeByBlockId(action.blockid);
            if (!leaf && model.treeState.rootNode) {
                leaf = findNodeByBlockId(model.treeState.rootNode, action.blockid);
            }
            let targetLeaf = model?.getNodeByBlockId(action.targetblockid);
            if (!targetLeaf && model.treeState.rootNode) {
                targetLeaf = findNodeByBlockId(model.treeState.rootNode, action.targetblockid);
            }
            if (!leaf?.data || !targetLeaf?.data) {
                console.error(
                    "Cannot apply layout action StackMove, source or target blockId not found",
                    action.blockid,
                    action.targetblockid
                );
                break;
            }
            const position = (action.position as "before" | "after" | "end" | undefined) ?? "end";
            if (leaf.id === targetLeaf.id) {
                moveMemberInStack(leaf.data, action.blockid, action.targetblockid, position, action.focused);
            } else {
                // Cross-pane move (Phase 4, SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md
                // §3.4) — `position` is irrelevant here (a cross-pane
                // header-drop always appends), same as `moveBlockInStack`'s
                // own cross-leaf branch.
                const result = moveMemberAcrossStacks(leaf.data, targetLeaf.data, action.blockid, action.focused);
                // The moved tab was its pane's only one: mirror the removal
                // of that emptied pane. A move, not a close — never
                // closeNode/onNodeDelete here, same reason as the DeleteNode
                // case above (R1 / #1681).
                // SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §4.2.
                if (result === "emptied") removeLeafEmptiedByMove(model, leaf.id);
            }
            break;
        }
        case LayoutTreeActionType.StackActivate: {
            const leaf = model?.getNodeByBlockId(action.blockid) ?? findNodeByBlockId(model.treeState.rootNode, action.blockid);
            if (!leaf?.data || !activateMemberInStack(leaf.data, action.blockid)) {
                console.error("Cannot apply layout action StackActivate, no pane holds blockId", action.blockid);
            }
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
            if (alreadyPlaced(model, action)) break;
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
                newNode: mirroredLeaf(action),
            };
            model.treeReducer(replaceAction, false, { actionId: action.actionid });
            break;
        }
        case LayoutTreeActionType.SplitHorizontal: {
            if (alreadyPlaced(model, action)) break;
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
            const newNode = mirroredLeaf(action);
            const splitAction: LayoutTreeSplitHorizontalAction = {
                type: LayoutTreeActionType.SplitHorizontal,
                targetNodeId: targetNode.id,
                newNode: newNode,
                position: action.position,
                sizeFraction: action.nodesizefraction,
                // A split-placed open (muxsh open/web, OpenEditor next to the
                // calling pane) selects the new pane exactly like an insert does.
                focused: action.focused,
            };
            model.treeReducer(splitAction, false, { actionId: action.actionid });
            break;
        }
        case LayoutTreeActionType.SplitVertical: {
            if (alreadyPlaced(model, action)) break;
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
            const newNode = mirroredLeaf(action);
            const splitAction: LayoutTreeSplitVerticalAction = {
                type: LayoutTreeActionType.SplitVertical,
                targetNodeId: targetNode.id,
                newNode: newNode,
                position: action.position,
                sizeFraction: action.nodesizefraction,
                focused: action.focused,
            };
            model.treeReducer(splitAction, false, { actionId: action.actionid });
            break;
        }
        default:
            console.warn("unsupported layout action", action);
            break;
    }
}

/**
 * Send the pane-tab command that mirrors a local edit this model already made
 * (`pane.activateTab`, `pane.removeTab`, `pane.moveTab`). srv applies it and
 * queues it for other views under the id minted here, which this model skips.
 * It leaves through the layout outbox, in order with the intents. A failure
 * is only logged (the returned promise never rejects): the window then shows
 * srv's layout. SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3.
 */
export function sendStackCommand(
    model: LayoutModel,
    op: string,
    send: (actionId: string) => Promise<PaneTabEditResult>
): Promise<void> {
    const actionId = crypto.randomUUID();
    model.processedActionIds.add(actionId);
    return new Promise((done) =>
        queueLayoutCommand(model, async () => {
            try {
                const res = await send(actionId);
                // Compare with srv only once the row shows this change.
                if (res?.version) model.layoutVersionFloor = Math.max(model.layoutVersionFloor, res.version);
            } catch (e) {
                console.warn(`[layout] ${op} failed; showing srv's layout`, e);
            } finally {
                done();
            }
        })
    );
}
