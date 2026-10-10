// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane-tab (pill) drag on the drag session, and its document-tab sibling: begun by the pill's onDragStart,
 * marked released by its onDrop, ended by the cross-window monitor's dragend,
 * which still reads `escaped` from it.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1.
 */

import { beginDrag, isUnderway, markReleased, session } from "./drag-session";

/** `paneKey` is the pane the pill belongs to; `tabId` its window tab. */
export function startPaneTabDrag(blockId: string, paneKey?: string, tabId?: string): void {
    beginDrag("pane-tab", { blockId, nodeId: paneKey, tabId });
}

export function releasePaneTabDrag(): void {
    if (session()?.kind === "pane-tab") markReleased();
}

/** A document tab (`tabId`, in block `blockId`'s strip) on the drag session.
 *  The same life as a pane-tab drag, without the tear-off. */
export function startDocTabDrag(blockId: string, tabId: string): void {
    beginDrag("doc-tab", { blockId, itemId: tabId });
}

export function releaseDocTabDrag(): void {
    if (session()?.kind === "doc-tab") markReleased();
}

/** Whether document tab `tabId` of block `blockId` is being dragged. Reactive. */
export function isDraggedDocTab(blockId: string, tabId: string): boolean {
    const source = session()?.source;
    return isUnderway("doc-tab") && source?.blockId === blockId && source?.itemId === tabId;
}

/**
 * Whether this pane's pill for `blockId` is being dragged, until its source
 * releases it. The pane matters: the same block can have a pill in more than
 * one strip. Reactive.
 */
export function isDraggedPaneTab(blockId: string, paneKey?: string): boolean {
    const source = session()?.source;
    return isUnderway("pane-tab") && source?.blockId === blockId && source?.nodeId === paneKey;
}
