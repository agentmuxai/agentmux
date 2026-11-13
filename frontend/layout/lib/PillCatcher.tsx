// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The layout's own drop target for a single pane tab that is being placed as a pane (pillPlacement.ts). It
 * covers the whole layout, above the panes, and takes the pointer only while a placement is on: the panes the
 * live reflow has moved are then not what the pointer is read against. Its own element, not the tile
 * overlay's: that one is a pane drag's, with its own rules for the dragged pane's place.
 */

import { dropTargetForElements } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { onCleanup, onMount } from "solid-js";
import { throttle } from "throttle-debounce";
import { paneTabItemType } from "@/app/drag/drag-types";
import type { LayoutModel } from "./layoutModel";
import { pillPlacingIn } from "./pillDragState";
import { dropPill, endPillPlacement, followPill } from "./pillPlacement";

/** As often as a pane drag is sampled (tilelayout-shared.tsx). */
const SAMPLE_MS = 50;

export function PillCatcher(props: { layoutModel: LayoutModel }) {
    let el: HTMLDivElement | undefined;
    const sample = throttle(SAMPLE_MS, (x: number, y: number) => followPill(props.layoutModel, { x, y }));
    onMount(() => {
        if (!el) return;
        const cleanup = dropTargetForElements({
            element: el,
            canDrop: ({ source }) => source.data.type === paneTabItemType && pillPlacingIn(props.layoutModel),
            onDrag: ({ location }) => sample(location.current.input.clientX, location.current.input.clientY),
            // Out of the layout (the window's tab bar, the window's edge): nothing is placed from there.
            onDragLeave: () => {
                sample.cancel({ upcomingOnly: true });
                endPillPlacement(props.layoutModel);
            },
            onDrop: ({ location }) => {
                sample.cancel({ upcomingOnly: true });
                dropPill(props.layoutModel, { x: location.current.input.clientX, y: location.current.input.clientY });
            },
        });
        onCleanup(() => {
            cleanup();
            sample.cancel();
        });
    });
    return <div ref={el} class="pill-catcher" style={{ "pointer-events": pillPlacingIn(props.layoutModel) ? "auto" : "none" }} />;
}
