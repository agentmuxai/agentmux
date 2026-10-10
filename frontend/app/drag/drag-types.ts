// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pragmatic-dnd drag tags, in one place.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1.
 */

export const tileItemType = "TILE_ITEM";
export const tabItemType = "TAB_ITEM";
export const paneTabItemType = "PANE_TAB_ITEM";
/** A document tab (an Editor's or a Media pane's file): moves only between
 *  panes of the same type, and never tears off (PaneTabStrip's `docDrag`). */
export const docTabItemType = "DOC_TAB_ITEM";

type Tagged = { type?: unknown } | Record<string, unknown> | null | undefined;

const tagOf = (data: Tagged) => (data as { type?: unknown } | null | undefined)?.type;

export const isTileSource = (data: Tagged) => tagOf(data) === tileItemType;
export const isTabSource = (data: Tagged) => tagOf(data) === tabItemType;
export const isPaneTabSource = (data: Tagged) => tagOf(data) === paneTabItemType;
export const isDocTabSource = (data: Tagged) => tagOf(data) === docTabItemType;

/** What a document-tab drag carries (pragmatic-dnd's `source.data`). */
export interface DocTabDragData {
    type: typeof docTabItemType;
    tabId: string;
    docType: string;
    sourceBlockId: string;
}

export function asDocTabDragData(data: Record<string | symbol, unknown>): DocTabDragData | null {
    return data.type === docTabItemType &&
        typeof data.tabId === "string" &&
        typeof data.docType === "string" &&
        typeof data.sourceBlockId === "string"
        ? (data as unknown as DocTabDragData)
        : null;
}
