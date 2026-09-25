// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { batch } from "solid-js";
import { fireAndForget } from "@/util/util";
import { isTileDragInFlight } from "./dragInFlight";
import { findNodeByBlockId, newLayoutNode, walkNodes } from "./layoutNode";
import { rebuildMinimizedSet } from "./layoutMinimize";
import { removeBlockFromLeaf, removeLeafEmptiedByMove } from "./layoutMagnify";
import { addMemberToStack, moveMemberAcrossStacks, moveMemberInStack } from "./stackMembers";
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

    if (initialState.pendingBackendActions?.length) {
        fireAndForget(() => processPendingBackendActions(model));
    } else {
        model.updateTree();
    }

    // One-shot startup heal: remove dangling leaves persisted by an earlier
    // session's races once Tab + LayoutState have both settled. Deliberately
    // a delayed single pass, not a reactive subscription — see the
    // onBackendUpdate note below and SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR §3.5.
    setTimeout(() => pruneDanglingLeaves(model), 2000);
}

/**
 * Handle a MuxObject update notification from the backend.
 * @param model The LayoutModel instance.
 */
export function onBackendUpdate(model: LayoutModel) {
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
    // NOTE deliberately NO prune here (SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR
    // §3.5): running it reactively on every Tab/LayoutState change made the
    // Tab object a tracked dependency and could delete a LEGITIMATE
    // freshly-created leaf whose block hadn't landed in tab.blockids yet
    // (the round-4 "drops don't stick" regression). Prune triggers are now:
    // model init (below), the tab bar's post-drag settle pass, and the
    // redock failure path.
}

// Age-gate registry (review finding on SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR
// PR #2105, P1): createBlock/createBlockSplitHorizontally/createBlockSplit-
// Vertically (global.ts) insert the new leaf into the local tree
// SYNCHRONOUSLY via treeReducer, but the block's membership in
// `tab.blockids` only lands later via an async MuxObject push. Any prune
// trigger that fires inside that window would see the fresh leaf as
// "disowned" and delete + persist the deletion — a real, distinct path to
// the same class of bug pruneDanglingLeaves exists to fix. Call sites for
// those three functions mark the new block here; pruning skips anything
// marked within the last RECENT_BLOCK_GRACE_MS.
const RECENT_BLOCK_GRACE_MS = 3000;
const recentlyCreatedBlocks = new Map<string, number>();

/** Call immediately after locally inserting a new block's leaf via treeReducer. */
export function markBlockRecentlyCreated(blockId: string, now: number = Date.now()): void {
    recentlyCreatedBlocks.set(blockId, now);
}

function isRecentlyCreated(blockId: string, now: number): boolean {
    const at = recentlyCreatedBlocks.get(blockId);
    if (at === undefined) return false;
    if (now - at > RECENT_BLOCK_GRACE_MS) {
        recentlyCreatedBlocks.delete(blockId);
        return false;
    }
    return true;
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
 * it is always safe FOR AN ALREADY-SETTLED leaf: this only ever REMOVES
 * leaves for disowned blocks. But a block just created via
 * createBlock/createBlockSplitHorizontally/createBlockSplitVertically is
 * inserted into the LOCAL tree before its `tab.blockids` membership
 * round-trips through the backend — such a leaf is age-gated via
 * `markBlockRecentlyCreated` so this function never deletes it out from
 * under the user (the round-4-class regression this review caught).
 *
 * Triggers (deliberately sparse — see the round-4 regression note in
 * onBackendUpdate): one-shot at model init (+2s), the tab bar's
 * post-drag settle pass, and the redock failure path. NOT reactive.
 */
export function pruneDanglingLeaves(model: LayoutModel) {
    const rootNode = model.treeState?.rootNode;
    const tab = model.tabAtom?.();
    if (!rootNode || !tab?.blockids) return;
    // Never prune while a pane drag is in flight: MoveBlock's Tab update
    // can reach this window BEFORE the drag's dragend dispatches (observed
    // 2ms apart in field logs), and deleting the drag-source leaf mid-drag
    // unmounts the source element — Chromium then never fires dragend on
    // it, pragmatic's teardown chain (activeDrag reset and monitor onDrop)
    // is skipped, and the source tab's overlay wedges at
    // pointer-events:auto. The flag (NOT currentDragPayload, which is
    // already cleared at drop time) spans the full gesture; the tab bar's
    // end-of-drag cleanup re-runs the prune once the drag has settled.
    if (isTileDragInFlight()) return;
    const now = Date.now();
    const owned = new Set(tab.blockids);
    const danglingIds: string[] = [];
    walkNodes(rootNode, (node) => {
        if (
            !node.children?.length &&
            node.data?.blockId &&
            !owned.has(node.data.blockId) &&
            !isRecentlyCreated(node.data.blockId, now)
        ) {
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
 * Process all pending backend actions from the MuxObject queue.
 * @param model The LayoutModel instance.
 */
export async function processPendingBackendActions(model: LayoutModel) {
    const muxObj = model.getter(model.muxObjectAtom);
    const actions = muxObj?.pendingbackendactions;
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
    // No prune here — a just-applied queued INSERT's block may not be in the
    // frontend's tab.blockids yet (SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR §3.5).
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
            // by processPendingBackendActions' updateTree + persist.
            if (leaf) {
                removeBlockFromLeaf(model, leaf, action.blockid);
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
            // agentmux-srv/src/server/service/layout_helpers.rs).
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
                // A split-placed open (muxsh open/web, OpenEditor next to the
                // calling pane) selects the new pane exactly like an insert does.
                focused: action.focused,
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
                focused: action.focused,
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
 * Persist current tree state to the backend MuxObject (debounced).
 * @param model The LayoutModel instance.
 */
export function persistToBackend(model: LayoutModel) {
    if (model.persistDebounceTimer) {
        clearTimeout(model.persistDebounceTimer);
    }

    model.persistDebounceTimer = setTimeout(() => {
        const muxObj = model.getter(model.muxObjectAtom);
        if (!muxObj) return;

        muxObj.rootnode = model.treeState.rootNode;
        muxObj.focusednodeid = model.treeState.focusedNodeId;
        muxObj.magnifiednodeid = model.treeState.magnifiedNodeId;
        muxObj.leaforder = model.treeState.leafOrder;
        // Persistence Phase A (SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR §3.6):
        // the pendingbackendactions queue is BACKEND-owned. Never write our
        // (often stale) local copy — a debounced persist racing a freshly
        // queued action used to erase it (the stale-tree resurrection
        // engine). Instead, carry forward the LIVE queue minus the actions
        // this model has already processed: ordinary persists preserve
        // unseen actions verbatim, and post-processing persists still clear
        // consumed ones.
        const liveQueue = model.getter(model.muxObjectAtom)?.pendingbackendactions;
        const unprocessed = liveQueue?.filter(
            (a) => a.actionid && !model.processedActionIds.has(a.actionid)
        );
        muxObj.pendingbackendactions = unprocessed?.length ? unprocessed : undefined;

        model.setter(model.muxObjectAtom, muxObj);
        model.persistDebounceTimer = null;
    }, 100);
}
