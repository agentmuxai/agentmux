// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A tile for a singleton view in a fresh pane, when a pane of that view is already open in this tab: the open pane
 * takes this pane's place (a layout swap, so it is the same block with all its state) and this pane, now where the
 * open one was, closes. The person asked for the view here, so it is here, not elsewhere.
 *
 * Only a lone pane moves: one in a stack of in-pane tabs would take its whole stack along, and one in another tab
 * of the window cannot be swapped with a pane of this tab. Those, and every other way of opening the view, keep
 * what they do (the tile brings the open pane back).
 */

import { getLayoutModelForStaticTab } from "@/layout/index";
import { effectiveStack } from "@/layout/lib/stackMembers";
import { LayoutTreeActionType, type LayoutTreeSwapNodeAction } from "@/layout/lib/types";
import { findOpenBlockByView, showBlockInPane } from "@/app/util/reveal-block";

/** What the move reads and does, so the decision can be tested without a window. */
export interface MoveHereDeps {
    /** The block of a pane of `view` open in this window, or null. */
    findOpen(view: string): Promise<string | null>;
    /** The layout leaf of a lone pane of this tab holding `blockId`; undefined for one in a stack, another tab or none. */
    loneNodeId(blockId: string): string | undefined;
    swap(nodeId1: string, nodeId2: string): void;
    /** Show and focus the pane `blockId` after the swap (a minimized one is restored). */
    show(blockId: string): void;
}

const realDeps: MoveHereDeps = {
    findOpen: (view) => findOpenBlockByView(view),
    loneNodeId(blockId) {
        const node = getLayoutModelForStaticTab().getNodeByBlockId(blockId);
        if (node?.id == null || node.data == null) return undefined;
        return effectiveStack(node.data).length > 1 ? undefined : node.id;
    },
    swap(nodeId1, nodeId2) {
        const action: LayoutTreeSwapNodeAction = { type: LayoutTreeActionType.Swap, node1Id: nodeId1, node2Id: nodeId2 };
        getLayoutModelForStaticTab().treeReducer(action);
    },
    show(blockId) {
        const model = getLayoutModelForStaticTab();
        const node = model.getNodeByBlockId(blockId);
        if (node?.id == null) return;
        showBlockInPane(model, node, blockId);
        model.focusNode(node.id);
    },
};

/**
 * Moves the open pane of `view` into the place of the pane `ownBlockId`, then runs `closeOwn` (this pane, now where
 * the open one was). False, with nothing changed, when there is no open pane to move here.
 */
export async function moveOpenPaneHere(
    ownBlockId: string,
    view: string,
    closeOwn: () => Promise<void>,
    deps: MoveHereDeps = realDeps
): Promise<boolean> {
    const openBlockId = await deps.findOpen(view);
    if (openBlockId == null || openBlockId === ownBlockId) return false;
    const ownNodeId = deps.loneNodeId(ownBlockId);
    const openNodeId = deps.loneNodeId(openBlockId);
    if (ownNodeId == null || openNodeId == null) return false;
    deps.swap(ownNodeId, openNodeId);
    await closeOwn();
    deps.show(openBlockId);
    return true;
}
