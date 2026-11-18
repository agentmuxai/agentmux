// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The Files pane's zoom parts: Places (the side list) has its own level, and everything else (the toolbar,
 * the list, the preview and the footer) keeps the pane's `term:zoom`. The shared mechanism is
 * `@/app/store/zoom-parts`; this is only what Files declares.
 * docs/specs/SPEC_EDITOR_TREE_AND_EDITOR_SEPARATE_ZOOM_2026_11_16.md §9.
 */

import type { PaneZoomCapability } from "@/app/store/pane-tab-registry";
import { paneZoom } from "@/app/store/pane-zoom-local";
import { subjectId, zoomPartsSeed } from "@/app/store/zoom-parts";

export const PLACES_PART = "places";
/** Block meta: Places' zoom level (`term:zoom` is the rest of the pane's). */
export const PLACES_ZOOM_KEY = "files:places_zoom";

export const FILES_ZOOM: PaneZoomCapability = {
    baseFontSize: 12,
    mainLabelKey: "zoom.part.files",
    parts: [{ id: PLACES_PART, metaKey: PLACES_ZOOM_KEY, labelKey: "zoom.part.places", baseFontSize: 12 }],
};

/** Places' level for a pane's meta (reactive where `meta` is). */
export const placesZoomOf = (blockId: string, meta: object | null | undefined): number =>
    paneZoom(subjectId(blockId, PLACES_PART), meta, PLACES_ZOOM_KEY);

/** First start with a Places level of its own: Places keeps the size it has had (the pane's zoom), once. */
export function seedPlacesZoom(meta: object | null | undefined, persist: (patch: Record<string, number>) => void): void {
    const seed = zoomPartsSeed(FILES_ZOOM.parts ?? [], meta);
    if (Object.keys(seed).length > 0) queueMicrotask(() => persist(seed));
}
