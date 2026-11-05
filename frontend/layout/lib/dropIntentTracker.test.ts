// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { DropIntentTracker, type Candidate } from "./dropIntentTracker";
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

    it("arms a candidate held for the dwell even while moving fast", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        tracker.step("d", 300, 100, 50, b(DropDirection.Left));
        const step = tracker.step("d", 600, 100, 200, b(DropDirection.Left));
        expect(step.armed).toEqual(b(DropDirection.Left));
    });

    it("arms the waiting candidate from a timer when no further sample comes", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        tracker.step("d", 300, 100, 50, b(DropDirection.Left));
        expect(tracker.flush(100)).toEqual(a(DropDirection.Left));
        expect(tracker.flush(170)).toEqual(b(DropDirection.Left));
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
        expect(tracker.flush(500)).toEqual(a(DropDirection.Left));
    });

    it("starts afresh for another dragged pane or after a long gap", () => {
        const tracker = new DropIntentTracker();
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        expect(tracker.step("e", 500, 100, 10, b(DropDirection.Top)).armed).toEqual(b(DropDirection.Top));
        expect(tracker.step("e", 900, 100, 5000, a(DropDirection.Top)).armed).toEqual(a(DropDirection.Top));
    });

    it("with no dwell, a candidate a margin away arms at once", () => {
        const tracker = new DropIntentTracker({ marginPx: 8, dwellMs: 0, slowPxPerMs: 0.35, staleMs: 1500 });
        tracker.step("d", 100, 100, 0, a(DropDirection.Left));
        expect(tracker.step("d", 300, 100, 50, b(DropDirection.Left)).armed).toEqual(b(DropDirection.Left));
    });
});
