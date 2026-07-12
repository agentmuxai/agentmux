// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { batch } from "solid-js";
import { fireAndForget } from "@/util/util";
import { isTileDragInFlight } from "./dragInFlight";
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

    // One-shot startup heal: remove dangling leaves persisted by an earlier
    // session's races once Tab + LayoutState have both settled. Deliberately
    // a delayed single pass, not a reactive subscription — see the
    // onBackendUpdate note below and SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR §3.5.
    setTimeout(() => pruneDanglingLeaves(model), 2000);
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
// `tab.blockids` only lands later via an async WaveObject push. Any prune
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
        // Persistence Phase A (SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR §3.6):
        // the pendingbackendactions queue is BACKEND-owned. Never write our
        // (often stale) local copy — a debounced persist racing a freshly
        // queued action used to erase it (the stale-tree resurrection
        // engine). Instead, carry forward the LIVE queue minus the actions
        // this model has already processed: ordinary persists preserve
        // unseen actions verbatim, and post-processing persists still clear
        // consumed ones.
        const liveQueue = model.getter(model.waveObjectAtom)?.pendingbackendactions;
        const unprocessed = liveQueue?.filter(
            (a) => a.actionid && !model.processedActionIds.has(a.actionid)
        );
        waveObj.pendingbackendactions = unprocessed?.length ? unprocessed : undefined;

        model.setter(model.waveObjectAtom, waveObj);
        model.persistDebounceTimer = null;
    }, 100);
}
