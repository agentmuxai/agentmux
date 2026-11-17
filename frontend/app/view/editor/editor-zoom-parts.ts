// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The Editor's zoom parts: the document keeps the pane's `term:zoom`, the file tree beside it has its own
 * level. The shared mechanism is `@/app/store/zoom-parts`; this is only what the Editor declares.
 * docs/specs/SPEC_EDITOR_TREE_AND_EDITOR_SEPARATE_ZOOM_2026_11_16.md.
 */

import { paneZoom } from "@/app/store/pane-zoom-local";
import type { PaneZoomCapability } from "@/app/store/pane-tab-registry";
import { subjectId, zoomPartsSeed } from "@/app/store/zoom-parts";

export const TREE_PART = "tree";
/** Block meta: the file tree's zoom level (`term:zoom` is the document's). */
export const TREE_ZOOM_KEY = "editor:tree_zoom";

export const EDITOR_ZOOM: PaneZoomCapability = {
    baseFontSize: 13,
    mainLabelKey: "zoom.part.editor",
    parts: [{ id: TREE_PART, metaKey: TREE_ZOOM_KEY, labelKey: "zoom.part.fileTree", baseFontSize: 13 }],
};

/** The file tree's level for a pane's meta (reactive where `meta` is). */
export const treeZoomOf = (blockId: string, meta: object | null | undefined): number =>
    paneZoom(subjectId(blockId, TREE_PART), meta, TREE_ZOOM_KEY);

/** First start with a tree level of its own: the tree keeps the size it has had (the pane's zoom), once. */
export function seedTreeZoom(meta: object | null | undefined, persist: (patch: Record<string, number>) => void): void {
    const seed = zoomPartsSeed(EDITOR_ZOOM.parts ?? [], meta);
    if (Object.keys(seed).length > 0) queueMicrotask(() => persist(seed));
}
