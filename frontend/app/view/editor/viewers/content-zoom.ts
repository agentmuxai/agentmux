// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * A viewer whose own content takes the pane's zoom (Ctrl+wheel, Ctrl +/-/0, the all-panes zoom), as
 * the browser pane's page does, while the Editor's chrome (tab strip, viewer bar, file tree) follows
 * chrome zoom. The text editor is not one: its zoom scales the whole pane. A viewer that is one
 * registers where its content scrolls and how to hold what is under the pointer, so the Editor's
 * zoom anchor can use it in place of CodeMirror's.
 * docs/specs/SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md §10.
 */

import type { ZoomAnchorAdapter } from "@/app/store/zoom-anchor";
import type { ViewerId } from "./viewer-choice";

const CONTENT_ZOOM_VIEWERS: ReadonlySet<ViewerId> = new Set<ViewerId>(["pdf"]);

export const hasContentZoom = (viewer: ViewerId): boolean => CONTENT_ZOOM_VIEWERS.has(viewer);

export interface ContentZoomHandle {
    /** What scrolls: a scroll by the reader ends a restore in progress. */
    scroller(): HTMLElement | undefined;
    adapter: ZoomAnchorAdapter;
}

const handles = new Map<string, ContentZoomHandle>();

/** The viewer in front for `blockId` takes the pane's zoom; returns the way to let go. */
export function registerContentZoom(blockId: string, handle: ContentZoomHandle): () => void {
    handles.set(blockId, handle);
    return () => {
        if (handles.get(blockId) === handle) handles.delete(blockId);
    };
}

export function contentZoomOf(blockId: string): ContentZoomHandle | undefined {
    return handles.get(blockId);
}
