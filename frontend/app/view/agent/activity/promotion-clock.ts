// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, type Accessor } from "solid-js";
import type { DocumentNode } from "../types";
import { nextToolPromotionAt } from "./tool-adapter";

/**
 * A reactive clock that ticks at the instant a running Bash call becomes due
 * for promotion to the ActivityDock (`TOOL_PROMOTION_MS`). Promotion crosses
 * its threshold on the wall clock, not on a document event, so anything that
 * depends on it reads this accessor to recompute right then.
 *
 * One `setTimeout` for the next-earliest promotion, not a continuous tick.
 * The scheduling effect reads its own tick, so when that timer fires it
 * reschedules for the next pending promotion; without that, two running calls
 * due at different times would only ever promote the earlier one.
 *
 * The one copy of what agent-view.tsx (twice) and ActivityDock.tsx each did
 * with their own nonce and timer
 * (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §5.2).
 */
export function createPromotionClock(nodes: Accessor<ReadonlyArray<DocumentNode>>): Accessor<number> {
    const [tick, setTick] = createSignal(0);
    createEffect(() => {
        tick();
        const now = Date.now();
        const at = nextToolPromotionAt(nodes(), now);
        if (at == null) return;
        const timer = setTimeout(() => setTick((n) => n + 1), Math.max(0, at - now) + 50);
        onCleanup(() => clearTimeout(timer));
    });
    return tick;
}
