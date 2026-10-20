// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dragging a whole pane out of the window by its header (the `"tile"` drag)
 * tears off the pane with ALL of its tabs, not only the visible one.
 * Shared by the three per-platform `CrossWindowDragMonitor`s.
 *
 * A pane that holds several pane tabs is one layout leaf with a `blockStack`;
 * `node.data.blockId` is only the visible tab. One pane-tab pill dragged out
 * on its own is a different payload (`pane-tab-tearoff.ts`) and moves just
 * that tab.
 *
 * RCA_WHOLE_PANE_TEAROFF_MOVES_ONLY_THE_VISIBLE_TAB_2026_10_19.md
 */

import { WorkspaceService } from "@/app/store/services";
import { redockFloaterPane } from "@/app/workspace/floater-panes";
import { removeMovedBlock } from "@/layout/lib/layoutMagnify";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { effectiveStack } from "@/layout/lib/stackMembers";
import type { LayoutNode } from "@/layout/lib/types";
import { Logger } from "@/util/logger";

/** What a header drag of a pane tears off: its visible tab and its whole tab
 *  stack in order (`[blockId]` for a pane with one tab). */
export interface PaneTearOff {
    blockId: string;
    members: string[];
}

/** The tear-off a `"tile"` drag of `node` stands for, or null when the node
 *  holds no block. */
export function paneTearOffOf(node: LayoutNode | undefined): PaneTearOff | null {
    const data = node?.data;
    const blockId = data?.blockId;
    if (!data || !blockId) return null;
    const stack = effectiveStack(data);
    return { blockId, members: stack.includes(blockId) ? [...stack] : [blockId] };
}

/** Take a torn-off pane's tabs out of the source layout once the floating
 *  window shows them. Each leaves its leaf (stack-safe), and the pane goes
 *  with the last; a no-op for a block the backend's queued `delete` already
 *  removed. */
export function removeTornOffPane(model: LayoutModel, members: string[]): void {
    for (const blockId of members) {
        removeMovedBlock(model, blockId);
    }
}

/**
 * Undo a tear-off whose floating window never opened: the backend already
 * moved every tab into a new workspace, so put them back as the pane they
 * were (one redock with the whole stack) and delete the now-empty workspace.
 * The pane's old position isn't recoverable (it is re-inserted), but no tab
 * is lost. If the source window tab was auto-closed (the pane was its only
 * one) there is nowhere to put them back: they stay in the new workspace,
 * reachable from the workspace switcher, and this is logged. Never throws.
 */
export async function rollBackPaneTearOff(
    pane: PaneTearOff,
    newWsId: string,
    sourceTabId: string,
    sourceWsId: string
): Promise<boolean> {
    try {
        const newWs = await WorkspaceService.GetWorkspace(newWsId);
        const floaterTabId = newWs?.tabids?.[0];
        if (!floaterTabId) throw new Error("the new workspace has no tab");
        await redockFloaterPane(
            { blockId: pane.members[0], members: pane.members, activeBlockId: pane.blockId },
            { tabId: floaterTabId, workspaceId: newWsId },
            { tabId: sourceTabId, workspaceId: sourceWsId }
        );
        await WorkspaceService.DeleteWorkspace(newWsId);
        Logger.warn("dnd:cross", "pane tear-off rolled back: the floating window didn't open", {
            blockId: pane.blockId,
            tabs: pane.members.length,
        });
        return true;
    } catch (e) {
        Logger.error("dnd:cross", "pane tear-off rollback failed — the pane is in its own workspace", {
            error: String(e),
            blockId: pane.blockId,
            newWsId,
        });
        return false;
    }
}
