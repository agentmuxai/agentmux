// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A document tab moved into a new pane of its type, beside another pane:
 * dropped near a pane's edge, or by "Move to New Pane" on the tab.
 *
 * The new pane is what a split of the tab's own pane makes (`splitBlockDefFor`:
 * an empty Editor on the same connection, an empty Media pane), split off the
 * target pane on that side. Once it has mounted and registered its host, the
 * tab moves through `dropDocTab`, as a tab dropped on an existing pane does,
 * with the same refusals; a refused move removes the empty pane again.
 * docs/reports/REPORT_DOC_TAB_DRAG_AND_DROP_2026_10_09.md §4 (Phase 4).
 */

import { dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { splitBlockDefFor } from "@/app/block/split-block-def";
import { asDocTabDragData } from "@/app/drag/drag-types";
import { notifyDrop } from "@/app/drag/file-drop-actions";
import { createBlockSplitHorizontally, createBlockSplitVertically, MOS } from "@/app/store/global";
import { getLayoutModelForStaticTab } from "@/layout/index";
import { docTabHost, dropDocTab, whenDocTabHost } from "./doc-tab-hosts";
import { docTabSplitSide, type SplitSide } from "./doc-tab-split-side";
import "./doc-tabs.scss";

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
    const split = side === "left" || side === "right" ? createBlockSplitHorizontally : createBlockSplitVertically;
    const newBlockId = await split(def, besideBlockId, side === "left" || side === "up" ? "before" : "after");
    if ((await whenDocTabHost(newBlockId)) && dropDocTab(sourceBlockId, tabId, newBlockId)) return true;
    // Refused after all, or the pane never came up: no empty pane left behind.
    const layout = getLayoutModelForStaticTab();
    const node = layout?.getNodeByBlockId(newBlockId);
    if (node) void layout.closeNode(node.id);
    return false;
}

const SPLIT_CLASS: Record<SplitSide, string> = {
    left: "doc-tab-split--left",
    right: "doc-tab-split--right",
    up: "doc-tab-split--up",
    down: "doc-tab-split--down",
};

/**
 * Make `tile`, a pane's tile in the layout, take a document tab dropped near
 * one of its edges: the tab moves into a new pane on that side. While one
 * hovers there, the tile shows where the new pane goes. `blockId` is the
 * pane's block. Returns the cleanup.
 */
export function registerDocTabSplitTarget(tile: HTMLElement, blockId: () => string | undefined): () => void {
    const show = (side: SplitSide | null) => {
        for (const [s, cls] of Object.entries(SPLIT_CLASS)) tile.classList.toggle(cls, s === side);
    };
    // Shown only while the tile is the innermost target: over a tab of a
    // strip near the top edge, the drop is that tab's (it lands beside it).
    const showAt = (location: { current: { dropTargets: { element: Element }[]; input: { clientX: number; clientY: number } } }) =>
        show(location.current.dropTargets[0]?.element === tile ? docTabSplitSide(tile, location.current.input.clientX, location.current.input.clientY) : null);
    const cleanup = dropTargetForElements({
        element: tile,
        canDrop: ({ source, input }) => !!asDocTabDragData(source.data) && !!blockId() && docTabSplitSide(tile, input.clientX, input.clientY) != null,
        onDragEnter: ({ location }) => showAt(location),
        onDrag: ({ location }) => showAt(location),
        onDragLeave: () => show(null),
        onDrop: ({ source, location }) => {
            show(null);
            // Only as the innermost target (a pane's own targets take the
            // drops they want first).
            if (location.current.dropTargets[0]?.element !== tile) return;
            const data = asDocTabDragData(source.data);
            const side = docTabSplitSide(tile, location.current.input.clientX, location.current.input.clientY);
            const target = blockId();
            if (!data || !side || !target) return;
            // On the next task: the move unmounts the dragged pill, the
            // drag's live source, while pragmatic-dnd is still dispatching.
            setTimeout(() => void moveDocTabToNewPane(data.sourceBlockId, data.tabId, target, side), 0);
        },
    });
    return () => {
        show(null);
        cleanup();
    };
}
