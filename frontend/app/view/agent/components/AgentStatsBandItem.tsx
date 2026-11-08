// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useBlockStats } from "@/app/hook/useBlockStats";
import { BlockStatsBadge } from "@/element/blockstats";
import { createEffect, createMemo, onCleanup } from "solid-js";
import { registerTopBandItem, TOP_BAND_PRIORITY } from "../top-band/top-band-registry";

/**
 * The pane's live CPU and memory readout as its top band `stats` item (capability `statsBadgeIn: "band"`, so the
 * block frame draws none over the pane's corner). The band gives it a box of its own, left of the controls: full
 * ("0.0% 66M") while the band has room, a processor icon and the CPU figure when it is tight, never dropped.
 * It joins the band once the first figures arrive, so the band is not drawn empty. Draws nothing itself.
 */
export function AgentStatsBandItem(props: { blockId: string }): null {
    const stats = useBlockStats(props.blockId);
    const has = createMemo(() => stats() != null);
    createEffect(() => {
        if (!has()) return;
        const dispose = registerTopBandItem(props.blockId, {
            id: "stats",
            slot: "stats",
            priority: TOP_BAND_PRIORITY.stats,
            full: (ctx) => (
                <BlockStatsBadge blockId={props.blockId} placement="band" form="full" measuring={ctx.measuring} />
            ),
            compact: (ctx) => (
                <BlockStatsBadge blockId={props.blockId} placement="band" form="compact" measuring={ctx.measuring} />
            ),
        });
        onCleanup(dispose);
    });
    return null;
}
