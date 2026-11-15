// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The drop guides of a drag over the layout (dropGuides.ts): the hot spots over the hovered pane, each one a
 * small picture of the layout its drop leaves, with the dragged pane filled in. Purely visual: the pointer is
 * read against the spots' geometry, never against these elements, so they take no pointer events. Above the
 * panes and the landing zone, in the layout container's coordinates, the same ones the layout's rects are in.
 */

import clsx from "clsx";
import { For, Show } from "solid-js";
import { guideIn, type GuidePane, type GuideSpot } from "./dropGuides";
import type { LayoutModel } from "./layoutModel";

const pct = (n: number): string => `${Math.round(n * 10000) / 100}%`;

export function DropGuides(props: { layoutModel: LayoutModel }) {
    const guide = () => guideIn(props.layoutModel);
    // The spots are the same objects for as long as the pointer stays over one pane (dropGuides.ts), so their
    // elements stay and only the active one's class changes.
    const spots = () => guide()?.spots ?? [];
    return (
        <div class="drop-guides" aria-hidden="true">
            <Show when={guide()}>
                <For each={spots()}>
                    {(spot: GuideSpot) => (
                        <div
                            class={clsx("drop-guide", guide()?.active === spot.direction && "active")}
                            data-pane-overlay
                            style={{
                                transform: `translate3d(${Math.round(spot.x - spot.width / 2)}px, ${Math.round(spot.y - spot.height / 2)}px, 0)`,
                                width: `${spot.width}px`,
                                height: `${spot.height}px`,
                            }}
                        >
                            <div class="drop-guide-map">
                                <For each={spot.panes}>
                                    {(pane: GuidePane) => (
                                        <div
                                            class={clsx("drop-guide-pane", pane.moved && "moved", pane.joined && "joined")}
                                            style={{ left: pct(pane.left), top: pct(pane.top), width: pct(pane.width), height: pct(pane.height) }}
                                        />
                                    )}
                                </For>
                            </div>
                        </div>
                    )}
                </For>
            </Show>
        </div>
    );
}
