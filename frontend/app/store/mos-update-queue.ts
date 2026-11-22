// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The single-object update events (`waveobj:update`) are applied a frame later, in slices, instead of one by one inside
 * the websocket task.
 *
 * Why: in the 0.60.37 host log one such event costs a mean of 25 ms (max 147 ms) while every other kind costs about 1 ms,
 * and a batch of 16 objects costs about the same as one object: the cost is the reactive flush the write triggers
 * (the Swarm list, the panes' readers), not the write. Applied one by one in separate tasks, each update paid that flush;
 * applied together, a run of updates pays it once. A burst that arrives in one stretch (a turn that touches every agent's
 * block) then blocked the page for 300 ms or more in one frame.
 *
 * What it does:
 * - an update waits in a map keyed by object; a newer update of the same object replaces the older one (the newer
 *   carries the higher version, which is all `updateMuxObject` keeps);
 * - at the next frame the pending updates are applied in one `batch()`, as many as the last slices say fit in
 *   FLUSH_BUDGET_MS, and the rest follow in the next frame, so input and paint run between slices;
 * - ORDERING: anything else that reads or writes the store (any other event, an RPC response that carries updates) calls
 *   `flushPendingMuxUpdates()` first, which applies everything pending at once, so no handler sees an object older than the
 *   update that arrived before the event it handles. The gain is therefore on runs of consecutive updates, which is what a
 *   burst is.
 *
 * `globalThis.__mosSyncUpdates = true` turns it off (apply at once, as before): the switch an A/B run flips on one build.
 */

import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";

/** The event that carries a single object update (`WpsEvent.MuxObjUpdate`). */
export const MUX_OBJ_UPDATE_EVENT = "waveobj:update";

/**
 * Events whose handlers do not read objects the pending updates carry, so they do not make the queue flush first: the
 * agent output stream and telemetry, which are the traffic a streaming fleet sends all the time (thousands a minute) and
 * which would otherwise apply every pending update at once and undo the grouping. Anything not listed flushes first.
 */
export const ORDER_INDEPENDENT_EVENTS: ReadonlySet<string> = new Set(["blockfile", "tool_chunk", "sysinfo", "blockstats"]);

/** The time one slice of updates (the writes and the reactive flush they cause) should stay within: a frame. */
export const FLUSH_BUDGET_MS = 16;
/** A slice that took this long, or longer, is logged: it is a burst the budget could not split (one object's flush). */
export const SLOW_SLICE_MS = 50;
/** At most one slow-slice line in this window. */
export const SLOW_SLICE_LOG_WINDOW_MS = 5_000;
/** What one update is assumed to cost before any slice has been timed. */
const INITIAL_UPDATE_MS = 1;

export interface MuxUpdateQueueDeps {
    /** Applies updates in one reactive flush (`batch()` around `updateMuxObject`). */
    applyBatch(updates: MuxObjUpdate[]): void;
    /** The clock, in ms. */
    now(): number;
    /** Runs `fn` at the next frame (or soon after when the window is hidden and frames do not run). */
    schedule(fn: () => void): void;
    /** A line for the log. */
    log(line: string): void;
}

const keyOf = (u: MuxObjUpdate): string => `${u.otype}:${u.oid}`;

export function createMuxUpdateQueue(deps: MuxUpdateQueueDeps) {
    const pending = new Map<string, MuxObjUpdate>();
    let scheduled = false;
    let perUpdateMs = INITIAL_UPDATE_MS;
    let lastSlowLogAt = -Infinity;

    /** Applies the oldest `count` pending updates in one flush and learns what they cost. */
    function applySlice(count: number): void {
        const slice: MuxObjUpdate[] = [];
        for (const [key, update] of pending) {
            if (slice.length >= count) break;
            pending.delete(key);
            slice.push(update);
        }
        if (slice.length === 0) return;
        const start = deps.now();
        try {
            deps.applyBatch(slice);
        } finally {
            const took = deps.now() - start;
            // The next slice is sized from this one: the reactive flush is what costs, and it follows the load now.
            perUpdateMs = Math.max(0.05, took / slice.length);
            if (took >= SLOW_SLICE_MS && start - lastSlowLogAt >= SLOW_SLICE_LOG_WINDOW_MS) {
                lastSlowLogAt = start;
                const kinds = new Map<string, number>();
                for (const u of slice) kinds.set(u.otype, (kinds.get(u.otype) ?? 0) + 1);
                const mix = [...kinds].map(([otype, n]) => `${otype}:${n}`).join(",");
                deps.log(`[perf] slow-mux-update-slice n=${slice.length} ms=${Math.round(took)} kinds=${mix} pending=${pending.size}`);
            }
        }
    }

    function flushSlice(): void {
        scheduled = false;
        const fit = Math.max(1, Math.floor(FLUSH_BUDGET_MS / perUpdateMs));
        applySlice(fit);
        if (pending.size > 0) schedulePending();
    }

    function schedulePending(): void {
        if (scheduled) return;
        scheduled = true;
        deps.schedule(flushSlice);
    }

    return {
        /** Queue an update; it is applied at the next frame, with the others pending then. */
        enqueue(update: MuxObjUpdate): void {
            if (update == null) return;
            if ((globalThis as { __mosSyncUpdates?: boolean }).__mosSyncUpdates === true) {
                deps.applyBatch([update]);
                return;
            }
            const key = keyOf(update);
            // Newest last: an object updated again moves to the end.
            pending.delete(key);
            pending.set(key, update);
            schedulePending();
        },
        /** Apply everything pending now, in arrival order. Call it before handling anything that must see those updates. */
        flushAll(): void {
            if (pending.size === 0) return;
            applySlice(pending.size);
        },
        pendingCount(): number {
            return pending.size;
        },
    };
}

/** The frame after this one; a hidden window runs no frames, so a timer is the fallback. */
function scheduleFrame(fn: () => void): void {
    let ran = false;
    const run = () => {
        if (ran) return;
        ran = true;
        fn();
    };
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(run);
    setTimeout(run, 100);
}

let applyBatchImpl: (updates: MuxObjUpdate[]) => void = () => {};

/** Set by mos.ts: how a batch of updates is applied to the store. */
export function setMuxUpdateApplier(fn: (updates: MuxObjUpdate[]) => void): void {
    applyBatchImpl = fn;
}

const queue = createMuxUpdateQueue({
    applyBatch: (updates) => applyBatchImpl(updates),
    now: () => performance.now(),
    schedule: scheduleFrame,
    log: (line) => console.warn(line),
});

export const enqueueMuxUpdate = queue.enqueue;
export const flushPendingMuxUpdates = queue.flushAll;
export const pendingMuxUpdateCount = queue.pendingCount;
