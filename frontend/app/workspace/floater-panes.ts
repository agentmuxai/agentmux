// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a floating window holds, grouped by pane, so a redock puts back whole
 * panes. A pane torn off with several pane tabs is ONE layout leaf with a tab
 * stack but several blocks in the floater's tab; redocking those one block at
 * a time would land each as a pane of its own.
 *
 * RCA_WHOLE_PANE_TEAROFF_MOVES_ONLY_THE_VISIBLE_TAB_2026_10_19.md
 */

import { WorkspaceService } from "@/app/store/services";
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { findNodeByBlockId } from "@/layout/lib/layoutNode";
import { effectiveStack } from "@/layout/lib/stackMembers";
import * as MOS from "@/app/store/mos";

/** One pane of a floater. `blockId` is its first tab (the one that lands as
 *  the pane when docked), `members` its tabs in order, `activeBlockId` the
 *  visible one. */
export interface FloaterPane {
    blockId: string;
    members: string[];
    activeBlockId: string;
}

/** The panes of the floater whose tab is `tabId`, in `tab.blockids` order.
 *  A block with no leaf in the layout (not yet laid out) is a one-tab pane. */
export function floaterPanes(tabId: string): FloaterPane[] {
    // Non-reactive read: callers are async handlers with no reactive owner.
    const blockIds = MOS.getObjectValue<Tab>(MOS.makeORef("tab", tabId))?.blockids ?? [];
    const root = getLayoutModelForTabById(tabId)?.treeState?.rootNode;
    const panes: FloaterPane[] = [];
    const covered = new Set<string>();
    for (const blockId of blockIds) {
        if (covered.has(blockId)) continue;
        const data = root ? findNodeByBlockId(root, blockId)?.data : undefined;
        const members = data ? effectiveStack(data).filter((m) => blockIds.includes(m)) : [];
        if (!data || members.length < 2) {
            covered.add(blockId);
            panes.push({ blockId, members: [blockId], activeBlockId: blockId });
            continue;
        }
        members.forEach((m) => covered.add(m));
        panes.push({
            blockId: members[0],
            members,
            activeBlockId: members.includes(data.blockId) ? data.blockId : members[0],
        });
    }
    return panes;
}

/** The block a floater's window state (maximize, "always on top") is keyed
 *  on: the first block of its tab. Stable whichever tab of the pane is
 *  visible, so the state does not seem to change when the user switches tabs.
 *  The header controls use it too (`blockframe.tsx`). Reactive: reads the tab
 *  object's atom. */
export function floaterPaneBlockId(tabId: string): string | undefined {
    return MOS.getMuxObjectAtom<Tab>(MOS.makeORef("tab", tabId))()?.blockids?.[0];
}

/** `RedockFloatingPane` for one floater pane: all of its tabs go, in order. A
 *  one-tab pane makes exactly the call it always did. */
export function redockFloaterPane(
    pane: FloaterPane,
    source: { tabId: string; workspaceId: string },
    target: { tabId: string; workspaceId: string },
    ghost?: { blockId?: string | null; dir?: number | null }
) {
    if (pane.members.length < 2) {
        return ghost
            ? WorkspaceService.RedockFloatingPane(
                  pane.blockId,
                  source.tabId,
                  source.workspaceId,
                  target.tabId,
                  target.workspaceId,
                  ghost.blockId ?? null,
                  ghost.dir ?? null
              )
            : WorkspaceService.RedockFloatingPane(
                  pane.blockId,
                  source.tabId,
                  source.workspaceId,
                  target.tabId,
                  target.workspaceId
              );
    }
    return WorkspaceService.RedockFloatingPane(
        pane.blockId,
        source.tabId,
        source.workspaceId,
        target.tabId,
        target.workspaceId,
        ghost?.blockId ?? null,
        ghost?.dir ?? null,
        false,
        pane.members,
        pane.activeBlockId
    );
}
