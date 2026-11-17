// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How often one agent pane's stream flushes: the rate the app-wide stream scheduler
 * (stream-scheduler.ts) is asked for, read afresh each frame.
 *
 *  - "hidden": the pane is mounted but its tab is not shown (a kept-alive stack member, see
 *    `isBlockDormant`). Nobody can see it, so it takes the slow, one-a-frame, last-in-line slot. Its
 *    data stays current, only later: the off-pane indicators that read its store keep updating.
 *  - "full": the focused pane, the one being read or typed into.
 *  - "background": any other pane that is shown, flushed at about 30 Hz.
 *
 * Before this, no pane gave a rate, so every pane flushed as "full" and a hidden pane took a frame
 * slot like the one being typed into.
 */

import { createEffect, untrack } from "solid-js";
import { getFocusedBlockId, isBlockDormant } from "@/app/store/block-component-registry";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { createStreamFlushQueue, type StreamFlushQueue } from "./stream-flush-queue";
import type { FlushRate } from "./stream-scheduler";

export function paneFlushRate(blockId: string): FlushRate {
    if (isBlockDormant(blockId)()) return "hidden";
    // The focus lookup resolves through the layout of the shown tab; a pane that cannot be looked
    // up there must not be slowed on a guess, so any failure reads as "full".
    try {
        return getFocusedBlockId() === blockId ? "full" : "background";
    } catch {
        return "full";
    }
}

/**
 * One pane's flush queue, flushing at the pane's rate. Call it in the hook's own body (it makes an effect). When the
 * pane's tab is shown again, what it gathered flushes at its new rate at once, not at the slow hidden one: asking
 * again re-arms the scheduler, which reads the rate afresh.
 */
export function createPaneFlushQueue(model: AgentPaneModel, blockId: string): StreamFlushQueue {
    const queue = createStreamFlushQueue(model, () => paneFlushRate(blockId));
    const dormant = isBlockDormant(blockId);
    createEffect(() => {
        // Untracked: asking reads every pending pane's rate (their dormancy, the focus), and a focus change must not re-ask.
        if (blockId && !dormant()) untrack(() => queue.scheduleFlush());
    });
    return queue;
}
