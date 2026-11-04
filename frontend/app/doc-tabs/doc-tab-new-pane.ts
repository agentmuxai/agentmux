// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A document tab moved into a new pane of its type, beside another pane, by
 * "Move to New Pane" on the tab. Dragging a document tab never does this: it
 * only lands on another document pane's tab bar (doc-tab-hosts.ts).
 *
 * The new pane is what a split of the tab's own pane makes (`splitBlockDefFor`:
 * an empty Editor on the same connection, an empty Media pane), split off the
 * target pane on that side. Once it has mounted and registered its host, the
 * tab moves through `dropDocTab`, as a tab dropped on an existing pane does,
 * with the same refusals; a refused move removes the empty pane again.
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md §4 (Phase 4).
 */

import { splitBlockDefFor } from "@/app/store/split-block-def";
import { notifyDrop } from "@/app/drag/file-drop-actions";
import { MOS } from "@/app/store/global";
import { createBlockSplitHorizontally, createBlockSplitVertically } from "@/app/store/block-layout-actions";
import { getLayoutModelForStaticTab } from "@/layout/index";
import { docTabHost, dropDocTab, whenDocTabHost } from "./doc-tab-hosts";
import "./doc-tabs.scss";

/** The side of the target pane the new pane goes on. */
export type SplitSide = "left" | "right" | "up" | "down";

/**
 * Move tab `tabId` of block `sourceBlockId` into a new pane on `side` of block
 * `besideBlockId` (the tab's own pane included). True when it moved.
 */
export async function moveDocTabToNewPane(sourceBlockId: string, tabId: string, besideBlockId: string, side: SplitSide): Promise<boolean> {
    const host = docTabHost(sourceBlockId);
    const tab = host?.peek(tabId);
    if (!host || !tab) return false;
    // Asked before a pane is made for it (a tab being saved stays).
    const reason = host.refuseGive(tab);
    if (reason) {
        notifyDrop.cantMove(tab.title, reason);
        return false;
    }
    const source = MOS.getObjectValue(MOS.makeORef("block", sourceBlockId)) as Block | undefined;
    const def = splitBlockDefFor(source, () => ({ meta: { view: source?.meta?.view } }));
    // The layout the split goes into, taken now, as the split takes it: the
    // window tab on screen may change while the new pane comes up.
    const layout = getLayoutModelForStaticTab();
    const split = side === "left" || side === "right" ? createBlockSplitHorizontally : createBlockSplitVertically;
    const newBlockId = await split(def, besideBlockId, side === "left" || side === "up" ? "before" : "after");
    if ((await whenDocTabHost(newBlockId)) && dropDocTab(sourceBlockId, tabId, newBlockId)) return true;
    // Refused after all, or the pane never came up: no empty pane left behind.
    const node = layout?.getNodeByBlockId(newBlockId);
    if (node) void layout.closeNode(node.id);
    return false;
}
