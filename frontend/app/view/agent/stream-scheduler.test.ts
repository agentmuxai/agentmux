// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    INPUT_WINDOW_MS,
    MAX_PANES_PER_FRAME_WHILE_INTERACTING,
    STARVATION_MS,
    __markInputForTests,
    __resetStreamSchedulerForTests,
    cancelStreamFlush,
    hasPendingStreamFlush,
    requestStreamFlush,
} from "./stream-scheduler";

// Manual frames and a controllable clock.
let frames: FrameRequestCallback[] = [];
let clock = 1000;
function frame(): void {
    const due = frames;
    frames = [];
    for (const cb of due) cb(clock);
}

beforeEach(() => {
    frames = [];
    clock = 1000;
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        frames.push(cb);
        return frames.length;
    });
    vi.stubGlobal("cancelAnimationFrame", () => {
        frames = [];
    });
    vi.spyOn(performance, "now").mockImplementation(() => clock);
    __resetStreamSchedulerForTests();
});
afterEach(() => {
    __resetStreamSchedulerForTests();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

describe("stream scheduler", () => {
    it("without user input, every pending pane flushes in the next frame (the old behaviour)", () => {
        const order: string[] = [];
        for (const id of ["a", "b", "c"]) requestStreamFlush(id, () => order.push(id));
        expect(order).toEqual([]);
        frame();
        expect(order).toEqual(["a", "b", "c"]);
        expect(frames).toHaveLength(0);
    });

    it("while the user is interacting, one pane flushes per frame, oldest request first", () => {
        const order: string[] = [];
        for (const id of ["a", "b", "c"]) requestStreamFlush(id, () => order.push(id));
        __markInputForTests();
        frame();
        expect(order).toEqual(["a"]);
        clock += 16;
        frame();
        expect(order).toEqual(["a", "b"]);
        clock += 16;
        frame();
        expect(order).toEqual(["a", "b", "c"]);
        expect(frames).toHaveLength(0);
    });

    it("a pane that flushed and asks again goes to the back of the queue (round-robin)", () => {
        const order: string[] = [];
        const req = (id: string) => requestStreamFlush(id, () => order.push(id));
        req("a");
        req("b");
        __markInputForTests();
        frame(); // a
        req("a"); // a streams more
        clock += 16;
        frame(); // b first: it has waited longer
        clock += 16;
        frame(); // then a
        expect(order).toEqual(["a", "b", "a"]);
    });

    it("once the oldest has waited STARVATION_MS, two panes flush per frame — never more", () => {
        const order: string[] = [];
        for (const id of ["a", "b", "c", "d"]) requestStreamFlush(id, () => order.push(id));
        __markInputForTests();
        frame(); // a
        clock += STARVATION_MS; // b, c, d have all now waited the full window
        __markInputForTests();
        frame();
        expect(order).toEqual(["a", "b", "c"]); // two, oldest first, not all three
        clock += 16;
        frame();
        expect(order).toEqual(["a", "b", "c", "d"]);
    });

    it(`sustained typing with 8 panes streaming: at most ${MAX_PANES_PER_FRAME_WHILE_INTERACTING} per frame, every pane served within the starvation delay + ceil(8/2) frames`, () => {
        const panes = ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7"];
        const lastServed = new Map<string, number>();
        let frameNo = 0;
        const request = (id: string) =>
            requestStreamFlush(id, () => {
                perFrame++;
                lastServed.set(id, frameNo);
                request(id); // every pane is streaming continuously
            });
        let perFrame = 0;
        for (const id of panes) request(id);
        let maxPerFrame = 0;
        let maxGap = 0;
        const prev = new Map<string, number>();
        for (frameNo = 1; frameNo <= 200; frameNo++) {
            __markInputForTests(); // the user never stops typing
            perFrame = 0;
            frame();
            maxPerFrame = Math.max(maxPerFrame, perFrame);
            for (const [id, f] of lastServed) {
                if (f === frameNo && prev.has(id)) maxGap = Math.max(maxGap, f - prev.get(id)!);
                if (f === frameNo) prev.set(id, f);
            }
            clock += 33; // ~30 fps under load
        }
        expect(maxPerFrame).toBeLessThanOrEqual(MAX_PANES_PER_FRAME_WHILE_INTERACTING);
        // The budget rises to 2 only once the oldest has waited STARVATION_MS
        // (ceil(100 / 33) = 4 frames here), then the queue drains 2 a frame.
        const frameMs = 33;
        expect(maxGap).toBeLessThanOrEqual(Math.ceil(STARVATION_MS / frameMs) + Math.ceil(panes.length / 2));
        expect(new Set(lastServed.keys()).size).toBe(panes.length);
    });

    it("interaction mode ends INPUT_WINDOW_MS after the last input", () => {
        const order: string[] = [];
        for (const id of ["a", "b", "c"]) requestStreamFlush(id, () => order.push(id));
        __markInputForTests();
        clock += INPUT_WINDOW_MS; // input is now stale
        frame();
        expect(order).toEqual(["a", "b", "c"]);
    });

    it("requests are idempotent per pane: keep their place and time, run the newest flush once", () => {
        const calls: string[] = [];
        requestStreamFlush("a", () => calls.push("a1"));
        requestStreamFlush("b", () => calls.push("b"));
        clock += 50;
        requestStreamFlush("a", () => calls.push("a2")); // still first in line
        __markInputForTests();
        frame();
        expect(calls).toEqual(["a2"]);
        expect(hasPendingStreamFlush("b")).toBe(true);
        expect(frames).toHaveLength(1); // exactly one frame armed
    });

    it("cancel withdraws a request; with nothing left, the frame is not armed", () => {
        const calls: string[] = [];
        requestStreamFlush("a", () => calls.push("a"));
        cancelStreamFlush("a");
        expect(hasPendingStreamFlush("a")).toBe(false);
        expect(frames).toHaveLength(0);
        frame();
        expect(calls).toEqual([]);
    });

    it("one pane's exception does not stop the others, and is still reported", async () => {
        const calls: string[] = [];
        const errors: unknown[] = [];
        const qm = vi.spyOn(globalThis, "queueMicrotask").mockImplementation((cb) => {
            try {
                cb();
            } catch (e) {
                errors.push(e);
            }
        });
        requestStreamFlush("a", () => {
            throw new Error("boom");
        });
        requestStreamFlush("b", () => calls.push("b"));
        frame();
        expect(calls).toEqual(["b"]);
        expect(errors).toHaveLength(1);
        expect((errors[0] as Error).message).toBe("boom");
        qm.mockRestore();
    });

    it("a real key event anywhere in the page counts as interaction", () => {
        const order: string[] = [];
        for (const id of ["a", "b"]) requestStreamFlush(id, () => order.push(id));
        document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "x", bubbles: true }));
        frame();
        expect(order).toEqual(["a"]);
    });
});
