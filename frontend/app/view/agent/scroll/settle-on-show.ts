// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, untrack, type Accessor } from "solid-js";
import { useAgentDormant } from "../agent-dormancy";

/**
 * When this agent pane is shown again after its tab was hidden, and it follows,
 * the content it got while hidden (laid out but not drawn) commits now: have
 * the one-way flow land each stage of that at the bottom instead of easing to
 * it (`OneWayFlow.settleOnShow`). A pane the user scrolled up keeps its place.
 * SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §3.1.
 */
export function useSettleOnShow(flow: { settleOnShow(): void } | null, following: Accessor<boolean>): void {
    if (!flow) return;
    const dormant = useAgentDormant();
    // The previous value by hand: `on(..., { defer: true })` doesn't record it
    // on its skipped first run, so the first show would read "wasn't hidden".
    let wasHidden = untrack(dormant);
    createEffect(() => {
        const hidden = dormant();
        if (wasHidden && !hidden && untrack(following)) flow.settleOnShow();
        wasHidden = hidden;
    });
}
