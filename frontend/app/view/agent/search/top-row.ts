// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The row at the top of the transcript's screen, read from the rows that are drawn. The layout slice knows only the
 * virtualised region: once the reader is in the streaming tail (the newest rows, drawn outside the virtualiser) it
 * has no row at the scroll position, and a search started there would start from the top of the document. Whatever
 * is on screen is drawn, in the virtualised region or the tail, so the DOM answers for both.
 */

/** The transcript's scroller inside a pane. */
const SCROLLER = ".agent-document";

/** The id of the first drawn row whose bottom edge is below the scroller's top edge, or null when none is. */
export function topVisibleNodeId(root: ParentNode | null | undefined): string | null {
    const scroller = root?.querySelector<HTMLElement>(SCROLLER);
    if (!scroller) return null;
    const top = scroller.getBoundingClientRect().top; // perf:allow-layout-read — once per query the person typed
    for (const row of scroller.querySelectorAll<HTMLElement>("[data-node-id]")) {
        if (row.getBoundingClientRect().bottom > top) return row.dataset.nodeId ?? null; // perf:allow-layout-read — same
    }
    return null;
}
