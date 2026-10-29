// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Recovering from dropped events: a window whose WebSocket falls behind has events dropped from
// the full lane (eventbus.rs) and is told with `ws:eventsdropped` (ws_resync.rs). Its objects are
// as stale as the last update it got, so it re-fetches them (`resyncMuxObjects`). A pane's live
// stream (terminal output, an agent's turn) is not an object; a gap in one is not recovered here.

import { resyncMuxObjects } from "./mos";

/** Wait this long after the event for more to arrive: a stall drops events in bursts. */
export const QUIET_MS = 500;
/** At least this long between two re-fetches, so a window that keeps falling behind isn't also hit by
 *  a re-fetch of everything it shows every time. */
export const MIN_GAP_MS = 10_000;

let timer: ReturnType<typeof setTimeout> | null = null;
let lastRunAt = -Infinity;

function scheduleResync(): void {
    if (timer != null) return;
    const wait = Math.max(QUIET_MS, lastRunAt + MIN_GAP_MS - Date.now());
    timer = setTimeout(() => {
        timer = null;
        lastRunAt = Date.now();
        resyncMuxObjects().then(
            (changed) => console.log(`[resync] srv dropped events; re-fetched the objects in use, ${changed} had changed`),
            (err) => console.log("[resync] re-fetch failed", err),
        );
    }, wait);
}

/**
 * Called with every message the WebSocket delivers (`initWshrpc`), before it is routed: srv writes
 * `ws:eventsdropped` straight to the socket, so it is caught here and needs no subscription.
 */
export function noteEventsDroppedMessage(
    msg: { command?: string; data?: { event?: string; data?: unknown } } | null,
): void {
    if (msg?.command === "eventrecv" && msg.data?.event === "ws:eventsdropped") scheduleResync();
}

/** Test seam: forget the schedule. */
export function __resetResync(): void {
    if (timer != null) clearTimeout(timer);
    timer = null;
    lastRunAt = -Infinity;
}
