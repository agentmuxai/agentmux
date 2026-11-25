// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import {
    FLUSH_BUDGET_MS,
    ORDER_INDEPENDENT_EVENTS,
    SLOW_SLICE_MS,
    createMuxUpdateQueue,
} from "./mos-update-queue";

const update = (oid: string, version: number, otype = "block", updatetype = "update"): MuxObjUpdate =>
    ({ updatetype, otype, oid, obj: { otype, oid, version } }) as unknown as MuxObjUpdate;

/** A queue on a fake clock: applying a batch costs `fixedMs + perUpdateMs * n`, and a frame runs when `runFrame` is called. */
function rig(cost: { fixedMs: number; perUpdateMs: number }) {
    let clock = 0;
    const batches: MuxObjUpdate[][] = [];
    const sliceMs: number[] = [];
    const lines: string[] = [];
    let frame: (() => void) | null = null;
    let held = false;
    const queue = createMuxUpdateQueue({
        applyBatch: (updates) => {
            batches.push(updates);
            const ms = cost.fixedMs + cost.perUpdateMs * updates.length;
            sliceMs.push(ms);
            clock += ms;
        },
        now: () => clock,
        schedule: (fn) => {
            frame = fn;
        },
        log: (line) => lines.push(line),
        held: () => held,
    });
    return {
        queue,
        batches,
        sliceMs,
        lines,
        framePending: () => frame != null,
        runFrame: () => {
            const fn = frame;
            frame = null;
            fn?.();
        },
        runAllFrames: () => {
            let guard = 0;
            while (frame != null && guard++ < 1000) {
                const fn = frame;
                frame = null;
                fn();
            }
        },
        advance: (ms: number) => {
            clock += ms;
        },
        setHeld: (on: boolean) => {
            held = on;
        },
    };
}

afterEach(() => {
    delete (globalThis as { __mosSyncUpdates?: boolean }).__mosSyncUpdates;
});

describe("mux update queue", () => {
    it("applies nothing in the message task: an update waits for the next frame", () => {
        const r = rig({ fixedMs: 1, perUpdateMs: 1 });
        r.queue.enqueue(update("a", 1));
        expect(r.batches).toHaveLength(0);
        expect(r.queue.pendingCount()).toBe(1);
        expect(r.framePending()).toBe(true);
        r.runFrame();
        expect(r.batches).toEqual([[update("a", 1)]]);
        expect(r.queue.pendingCount()).toBe(0);
    });

    it("a run of updates arriving before the frame is one reactive flush, not one each", () => {
        const r = rig({ fixedMs: 20, perUpdateMs: 0.5 });
        for (let i = 0; i < 12; i++) r.queue.enqueue(update(`b${i}`, 1));
        r.runAllFrames();
        // 12 objects in one batch: 20 + 6 ms, where 12 separate flushes would have cost 12 * 20.5 ms.
        expect(r.batches).toHaveLength(1);
        expect(r.batches[0]).toHaveLength(12);
        expect(r.sliceMs[0]).toBeLessThan(FLUSH_BUDGET_MS * 2);
    });

    it("a newer update of the same object replaces the older, and an object updated again moves to the end", () => {
        const r = rig({ fixedMs: 1, perUpdateMs: 0.1 });
        r.queue.enqueue(update("a", 1));
        r.queue.enqueue(update("b", 1));
        r.queue.enqueue(update("a", 2));
        r.runAllFrames();
        expect(r.batches).toEqual([[update("b", 1), update("a", 2)]]);
    });

    it("a delete after an update leaves only the delete", () => {
        const r = rig({ fixedMs: 1, perUpdateMs: 0.1 });
        r.queue.enqueue(update("a", 1));
        r.queue.enqueue({ updatetype: "delete", otype: "block", oid: "a" } as unknown as MuxObjUpdate);
        r.runAllFrames();
        expect(r.batches).toHaveLength(1);
        expect(r.batches[0]).toHaveLength(1);
        expect(r.batches[0][0].updatetype).toBe("delete");
    });

    it("a burst that costs more than a frame is split across frames, and no slice after the first exceeds the budget", () => {
        // 30 objects, 4 ms each, no fixed cost: 120 ms if applied together.
        const r = rig({ fixedMs: 0, perUpdateMs: 4 });
        for (let i = 0; i < 30; i++) r.queue.enqueue(update(`c${i}`, 1));
        r.runAllFrames();
        expect(r.batches.flat()).toHaveLength(30);
        expect(r.batches.length).toBeGreaterThan(3);
        // The first slice has nothing to learn from; every one after it is sized from the last.
        for (const ms of r.sliceMs.slice(1)) expect(ms).toBeLessThanOrEqual(FLUSH_BUDGET_MS);
        // Arrival order is kept across slices.
        expect(r.batches.flat().map((u) => u.oid)).toEqual(Array.from({ length: 30 }, (_, i) => `c${i}`));
    });

    it("an object that alone costs more than the budget still gets applied, one at a time", () => {
        const r = rig({ fixedMs: 0, perUpdateMs: 40 });
        // One update first, so the queue has timed what an update costs here.
        r.queue.enqueue(update("learn", 1));
        r.runAllFrames();
        for (let i = 0; i < 3; i++) r.queue.enqueue(update(`d${i}`, 1));
        r.runAllFrames();
        expect(r.batches.flat()).toHaveLength(4);
        expect(r.batches.slice(1).map((b) => b.length)).toEqual([1, 1, 1]);
    });

    it("flushAll applies everything pending at once, in arrival order, before the frame", () => {
        const r = rig({ fixedMs: 0, perUpdateMs: 4 });
        for (let i = 0; i < 10; i++) r.queue.enqueue(update(`e${i}`, 1));
        r.queue.flushAll();
        expect(r.queue.pendingCount()).toBe(0);
        expect(r.batches).toHaveLength(1);
        expect(r.batches[0].map((u) => u.oid)).toEqual(Array.from({ length: 10 }, (_, i) => `e${i}`));
        r.queue.flushAll(); // nothing pending: nothing applied
        expect(r.batches).toHaveLength(1);
        r.runAllFrames(); // the frame that was scheduled finds nothing
        expect(r.batches).toHaveLength(1);
    });

    it("the lab switch applies an update at once, as before", () => {
        const r = rig({ fixedMs: 1, perUpdateMs: 1 });
        (globalThis as { __mosSyncUpdates?: boolean }).__mosSyncUpdates = true;
        r.queue.enqueue(update("a", 1));
        expect(r.batches).toEqual([[update("a", 1)]]);
        expect(r.framePending()).toBe(false);
    });

    it("a slice that took SLOW_SLICE_MS or more is logged once in a window, with what it held", () => {
        const r = rig({ fixedMs: 0, perUpdateMs: 60 });
        r.queue.enqueue(update("a", 1, "block"));
        r.queue.enqueue(update("l", 1, "layout"));
        r.runAllFrames();
        expect(r.lines).toHaveLength(1);
        expect(r.lines[0]).toMatch(/^\[perf\] slow-mux-update-slice n=2 ms=120 kinds=block:1,layout:1 pending=0$/);
        // Another slow slice inside the window adds no line.
        r.queue.enqueue(update("b", 1));
        r.runAllFrames();
        expect(r.lines).toHaveLength(1);
        // After the window it does.
        r.advance(10_000);
        r.queue.enqueue(update("c", 1));
        r.runAllFrames();
        expect(r.lines).toHaveLength(2);
        expect(SLOW_SLICE_MS).toBeGreaterThan(FLUSH_BUDGET_MS);
    });

    it("the output stream and telemetry leave updates pending; everything else flushes first", () => {
        for (const kind of ["blockfile", "tool_chunk", "sysinfo", "blockstats"]) expect(ORDER_INDEPENDENT_EVENTS.has(kind)).toBe(true);
        for (const kind of ["waveobj:batchedupdates", "config", "agentruntime:changed", "layout:action"]) {
            expect(ORDER_INDEPENDENT_EVENTS.has(kind)).toBe(false);
        }
    });
});

describe("a window-tab switch in flight", () => {
    it("keeps a frame queued before the click from applying updates until the hold ends", () => {
        const r = rig({ fixedMs: 1, perUpdateMs: 1 });
        r.queue.enqueue(update("a", 1));
        r.setHeld(true);
        r.runFrame();
        expect(r.batches).toHaveLength(0);
        expect(r.framePending()).toBe(true);
        r.setHeld(false);
        r.runFrame();
        expect(r.batches).toHaveLength(1);
    });
});
