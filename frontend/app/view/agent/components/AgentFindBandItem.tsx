// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, onCleanup } from "solid-js";
import { registerTopBandItem, TOP_BAND_PRIORITY } from "../top-band/top-band-registry";
import { AgentSearchBar, type AgentSearchBarProps } from "./AgentSearchBar";

/**
 * The find bar as the pane's top band `controls` item: it is in the band while the search is open, and the band
 * gives it its own box, so nothing else the band holds is drawn over it. Draws nothing itself.
 */
export function AgentFindBandItem(props: AgentSearchBarProps & { blockId: string }): null {
    createEffect(() => {
        if (!props.visible()) return;
        const dispose = registerTopBandItem(props.blockId, {
            id: "find",
            slot: "controls",
            priority: TOP_BAND_PRIORITY.controls,
            full: (ctx) => (
                <AgentSearchBar
                    visible={() => true}
                    onSearch={props.onSearch}
                    onNext={props.onNext}
                    onPrev={props.onPrev}
                    onClose={props.onClose}
                    matchIndex={props.matchIndex}
                    matchCount={props.matchCount}
                    searching={props.searching}
                    dim={props.dim}
                    onDimChange={props.onDimChange}
                    measuring={ctx.measuring}
                />
            ),
        });
        onCleanup(dispose);
    });
    return null;
}
