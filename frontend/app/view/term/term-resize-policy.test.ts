// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { createRefitScheduler, LIVE_REFIT_LARGE_BUFFER_LINES, liveRefit } from "./term-resize-policy";

describe("liveRefit (SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md §4.1)", () => {
    const cur = { cols: 80, rows: 24 };

    it("does nothing when the grid already fits", () => {
        expect(liveRefit({ cols: 80, rows: 24 }, cur, 10_000)).toBe("none");
    });

    it("applies a row change immediately, whatever the buffer", () => {
        expect(liveRefit({ cols: 80, rows: 30 }, cur, 10_000)).toBe("now");
    });

    it("applies a column change immediately on a short buffer", () => {
        expect(liveRefit({ cols: 100, rows: 24 }, cur, LIVE_REFIT_LARGE_BUFFER_LINES)).toBe("now");
    });

    it("throttles a column change on a long buffer instead of freezing it", () => {
        expect(liveRefit({ cols: 100, rows: 24 }, cur, LIVE_REFIT_LARGE_BUFFER_LINES + 1)).toBe("throttle");
        expect(liveRefit({ cols: 100, rows: 30 }, cur, LIVE_REFIT_LARGE_BUFFER_LINES + 1)).toBe("throttle");
    });
});

describe("createRefitScheduler", () => {
    // A fake frame clock: `frame()` runs what was requested for the next frame.
    const clock = () => {
        let pending: (() => void) | null = null;
        return {
            request: (cb: () => void) => {
                pending = cb;
            },
            frame: () => {
                const cb = pending;
                pending = null;
                cb?.();
            },
            requested: () => pending != null,
        };
    };

    it("runs up to the per-frame limit now and the rest on later frames, in order", () => {
        const c = clock();
        const s = createRefitScheduler(2, c.request);
        const ran: string[] = [];
        const keys = [{}, {}, {}, {}, {}];
        keys.forEach((k, i) => s.schedule(k, () => ran.push(`t${i}`)));
        expect(ran).toEqual(["t0", "t1"]);
        c.frame();
        expect(ran).toEqual(["t0", "t1", "t2", "t3"]);
        c.frame();
        expect(ran).toEqual(["t0", "t1", "t2", "t3", "t4"]);
        c.frame();
        expect(c.requested()).toBe(false);
    });

    it("keeps one job per key: a newer one replaces the waiting one in its place", () => {
        const c = clock();
        const s = createRefitScheduler(1, c.request);
        const [a, b] = [{}, {}];
        const ran: string[] = [];
        s.schedule({}, () => ran.push("first"));
        s.schedule(a, () => ran.push("a-old"));
        s.schedule(b, () => ran.push("b"));
        s.schedule(a, () => ran.push("a-new"));
        c.frame();
        c.frame();
        expect(ran).toEqual(["first", "a-new", "b"]);
    });

    it("counts a job that a running job schedules against the same frame", () => {
        const c = clock();
        const s = createRefitScheduler(2, c.request);
        const k = {};
        const ran: string[] = [];
        s.schedule(k, () => {
            ran.push("live");
            s.schedule(k, () => ran.push("cols"));
        });
        s.schedule({}, () => ran.push("other"));
        expect(ran).toEqual(["live", "cols"]);
        c.frame();
        expect(ran).toEqual(["live", "cols", "other"]);
    });

    it("drops a cancelled job", () => {
        const c = clock();
        const s = createRefitScheduler(1, c.request);
        const k = {};
        const ran: string[] = [];
        s.schedule({}, () => ran.push("now"));
        s.schedule(k, () => ran.push("cancelled"));
        s.cancel(k);
        c.frame();
        expect(ran).toEqual(["now"]);
    });
});
