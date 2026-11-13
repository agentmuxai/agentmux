// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { DEFAULT_TRACKER, DropIntentTracker, type Candidate } from "./dropIntentTracker";
import { DropDirection } from "./types";

const a = (direction: DropDirection): Candidate => ({ targetId: "A", direction });
const b = (direction: DropDirection): Candidate => ({ targetId: "B", direction });

describe("DropIntentTracker", () => {
    it("arms the first answer at once", () => {
        const tracker = new DropIntentTracker();
        expect(tracker.step("d", 100, 100, 0, a(DropDirection.Left)).armed).toEqual(a(DropDirection.Left));
    });

    it("keeps the armed candidate while the pointer jitters across a boundary", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        let switches = 0;
        let shown = a(DropDirection.Left);
        for (let i = 1; i <= 40; i++) {
            const raw = i % 2 ? a(DropDirection.Top) : a(DropDirection.Left);
            const x = 100 + (i % 2 ? 3 : -3);
            const armed = tracker.step("d", x, 100, i * 50, raw).armed;
            if (armed.direction !== shown.direction) switches++;
            shown = armed;
        }
        expect(switches).toBe(0);
    });

    it("moves to a new candidate once the pointer is a margin away and aiming slowly", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        const step = tracker.step("d", 112, 100, 100, a(DropDirection.Right));
        expect(step.armed).toEqual(a(DropDirection.Right));
    });

    it("waits out a fast pass over another candidate", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        const step = tracker.step("d", 300, 100, 50, b(DropDirection.Left));
        expect(step.armed).toEqual(a(DropDirection.Left));
        expect(step.flushAt).toBe(50 + 120);
    });

    it("arms a candidate held for the dwell while the pointer moves at an aiming pace", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        // 0.6 px/ms: faster than slow, slower than fast.
        tracker.step("d", 130, 100, 50, b(DropDirection.Left));
        const step = tracker.step("d", 220, 100, 200, b(DropDirection.Left));
        expect(step.armed).toEqual(b(DropDirection.Left));
    });

    it("a fast sweep arms nothing from its samples, however long it lasts", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        let last: ReturnType<DropIntentTracker["step"]> | undefined;
        // 2 px/ms across the window for a second, over pane B and then C.
        for (let i = 1; i <= 20; i++) {
            const raw = i < 10 ? b(DropDirection.Left) : { targetId: "C", direction: DropDirection.Left };
            last = tracker.step("d", 100 + i * 100, 100, i * 50, raw);
            expect(last.armed).toEqual(a(DropDirection.Left));
        }
        // Each sample pushes the dwell out: the candidate arms only once the samples stop.
        expect(last!.flushAt).toBe(1000 + 120);
        expect(tracker.armWaiting()).toEqual({ targetId: "C", direction: DropDirection.Left });
    });

    it("after a sweep, slowing down arms at once", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        for (let i = 1; i <= 10; i++) tracker.step("d", 100 + i * 100, 100, i * 50, b(DropDirection.Left));
        // Two slow samples bring the speed estimate under the aiming pace.
        tracker.step("d", 1102, 100, 600, b(DropDirection.Left));
        const step = tracker.step("d", 1104, 100, 700, b(DropDirection.Left));
        expect(step.armed).toEqual(b(DropDirection.Left));
    });

    it("arms the waiting candidate when the dwell timer fires with no further sample", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        tracker.step("d", 300, 100, 50, b(DropDirection.Left));
        expect(tracker.armWaiting()).toEqual(b(DropDirection.Left));
        expect(tracker.armWaiting()).toEqual(b(DropDirection.Left));
    });

    it("a different waiting candidate restarts the dwell", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        tracker.step("d", 300, 100, 50, b(DropDirection.Left));
        const step = tracker.step("d", 500, 100, 150, b(DropDirection.Top));
        expect(step.armed).toEqual(a(DropDirection.Left));
        expect(step.flushAt).toBe(150 + 120);
    });

    it("returning to the armed candidate cancels the wait", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        tracker.step("d", 300, 100, 50, b(DropDirection.Left));
        tracker.step("d", 110, 100, 100, a(DropDirection.Left));
        expect(tracker.armWaiting()).toEqual(a(DropDirection.Left));
    });

    it("starts afresh for another dragged pane or after a long gap", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        expect(tracker.step("e", 500, 100, 10, b(DropDirection.Top)).armed).toEqual(b(DropDirection.Top));
        expect(tracker.step("e", 900, 100, 5000, a(DropDirection.Top)).armed).toEqual(a(DropDirection.Top));
    });

    it("with no dwell, a candidate a margin away arms at once", () => {
        const tracker = new DropIntentTracker({ marginPx: 8, dwellMs: 0, slowPxPerMs: 0.35, fastPxPerMs: Infinity, staleMs: 1500 });
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        expect(tracker.step("d", 300, 100, 50, b(DropDirection.Left)).armed).toEqual(b(DropDirection.Left));
    });

    it("with the fast gate off, a fast pointer arms by dwell as before", () => {
        const tracker = new DropIntentTracker({ ...DEFAULT_TRACKER, fastPxPerMs: Infinity });
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        tracker.step("d", 300, 100, 50, b(DropDirection.Left));
        expect(tracker.step("d", 600, 100, 200, b(DropDirection.Left)).armed).toEqual(b(DropDirection.Left));
    });
});
