// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    INPUT_WINDOW_MS,
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

    it("the starvation guard flushes any pane that has waited STARVATION_MS, even while interacting", () => {
        const order: string[] = [];
        for (const id of ["a", "b", "c", "d"]) requestStreamFlush(id, () => order.push(id));
        __markInputForTests();
        frame(); // a
        clock += STARVATION_MS; // b, c, d have now waited the full window
        __markInputForTests();
        frame();
        expect(order).toEqual(["a", "b", "c", "d"]);
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
