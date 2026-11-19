// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Whether the page was frozen during a drag (drag-stall.ts): the verdict that stops a freeze from ending as a tear-off.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dragWasStalled, resetDragStallForTest, startDragStallWatch, STALL_MS } from "./drag-stall";

let now = 0;

/**
 * A real press. The module's own listener only takes a trusted `pointerdown`, which a test cannot make (jsdom
 * will not let an event say it is trusted), so the press is what that listener does.
 */
function press(): void {
    startDragStallWatch();
}

beforeEach(() => {
    vi.useFakeTimers();
    now = 1000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    resetDragStallForTest();
});

afterEach(() => {
    resetDragStallForTest();
    vi.restoreAllMocks();
    vi.useRealTimers();
});

/** Time passes with the page running: the heartbeat keeps up. */
async function runFor(ms: number): Promise<void> {
    for (let t = 0; t < ms; t += 200) {
        now += 200;
        await vi.advanceTimersByTimeAsync(200);
    }
}

describe("a drag the page could keep up with", () => {
    it("is not a stall", async () => {
        press();
        await runFor(5000);
        expect(dragWasStalled()).toBe(false);
    });

    it("is not a stall for a pause shorter than the limit", async () => {
        press();
        now += STALL_MS - 1;
        expect(dragWasStalled()).toBe(false);
    });

    it("is nothing before any press", () => {
        now += 60_000;
        expect(dragWasStalled()).toBe(false);
    });
});

describe("a freeze during a press or drag", () => {
    it("is seen by whoever asks first once it is over, before the heartbeat has run again", () => {
        press();
        // The page could not run for 16 s: no timer fired, then the queued dragend asks.
        now += 16_000;
        expect(dragWasStalled()).toBe(true);
    });

    it("is kept when the heartbeat is the one that runs first", async () => {
        press();
        now += 16_000;
        await vi.advanceTimersByTimeAsync(200);
        expect(dragWasStalled()).toBe(true);
        await runFor(1000);
        expect(dragWasStalled()).toBe(true);
    });

    it("is settled by the drag's end, and the verdict outlives the watch", () => {
        press();
        now += STALL_MS;
        document.dispatchEvent(new Event("dragend"));
        now += 60_000;
        expect(dragWasStalled()).toBe(true);
        expect(vi.getTimerCount()).toBe(0);
    });

    it("is settled by the release of a press that was only a click", () => {
        press();
        now += STALL_MS + 1;
        document.dispatchEvent(new Event("pointerup"));
        expect(dragWasStalled()).toBe(true);
    });

    it("is forgotten at the next press", async () => {
        press();
        now += 16_000;
        expect(dragWasStalled()).toBe(true);
        press();
        expect(dragWasStalled()).toBe(false);
        await runFor(1000);
        expect(dragWasStalled()).toBe(false);
    });
});

describe("what is watched", () => {
    it("ignores an event a script made", () => {
        document.dispatchEvent(new Event("pointerdown"));
        now += 60_000;
        expect(dragWasStalled()).toBe(false);
        expect(vi.getTimerCount()).toBe(0);
    });

    it("stops watching a press whose end is never seen", async () => {
        startDragStallWatch();
        expect(vi.getTimerCount()).toBeGreaterThan(0);
        await vi.advanceTimersByTimeAsync(121_000);
        expect(vi.getTimerCount()).toBe(0);
    });

    it("runs one heartbeat however many presses there are", () => {
        press();
        press();
        press();
        expect(vi.getTimerCount()).toBe(2); // the heartbeat and its cap
    });
});
