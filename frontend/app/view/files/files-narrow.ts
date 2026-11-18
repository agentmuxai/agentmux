// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What a narrow Files pane has no room for. Up to these widths of the pane there is no room for Places, or for
 * the preview, beside the list: the panel is not drawn, and neither is the toolbar button that would show it
 * (files.scss `data-no-places`, `data-no-preview`). The widths are counted in the pane's own zoomed pixels, as
 * they were when the whole pane was zoomed: a pane zoomed to 200% drops them at twice the width. The pane's root
 * is not zoomed now (its parts are, apart), so its width is divided by the main level here, not measured by a
 * container query. docs/specs/SPEC_EDITOR_TREE_AND_EDITOR_SEPARATE_ZOOM_2026_11_16.md §9.
 */

import { createEffect, createSignal, onCleanup, onMount, type Accessor } from "solid-js";

/** Up to this width there is no room for Places beside the list. */
export const PLACES_HIDDEN_TO = 480;
/** Up to this width there is no room for the preview, either. */
export const PREVIEW_HIDDEN_TO = 640;

/** What is hidden: `true` for a part with no room. */
export interface NarrowTiers {
    places: boolean;
    preview: boolean;
}

/** The tiers a pane `width` px wide (unzoomed) falls in at the main level `zoom`. A width not measured yet (0) hides nothing. */
export function narrowTiersFor(width: number, zoom: number): NarrowTiers {
    if (!(width > 0)) return { places: false, preview: false };
    const own = zoom > 0 ? width / zoom : width;
    return { places: own <= PLACES_HIDDEN_TO, preview: own <= PREVIEW_HIDDEN_TO };
}

const sameTiers = (a: NarrowTiers, b: NarrowTiers): boolean => a.places === b.places && a.preview === b.preview;

/** The tiers of the element `root()` returns, kept as it is resized and as `zoom()` changes. */
export function useNarrowTiers(root: () => HTMLElement | undefined, zoom: () => number): Accessor<NarrowTiers> {
    const [tiers, setTiers] = createSignal<NarrowTiers>({ places: false, preview: false }, { equals: sameTiers });
    let width = 0;
    const apply = (): void => {
        setTiers(narrowTiersFor(width, zoom()));
    };
    onMount(() => {
        const el = root();
        if (!el) return;
        width = el.clientWidth;
        apply();
        if (typeof ResizeObserver === "undefined") return;
        const observer = new ResizeObserver(() => {
            width = el.clientWidth;
            apply();
        });
        observer.observe(el);
        onCleanup(() => observer.disconnect());
    });
    createEffect(apply);
    return tiers;
}
