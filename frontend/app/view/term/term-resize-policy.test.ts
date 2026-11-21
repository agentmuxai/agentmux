// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    createRefitScheduler,
    LIVE_REFIT_LARGE_BUFFER_LINES,
    LIVE_REFIT_QUIET_MS,
    liveRefit,
} from "./term-resize-policy";

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

// The scheduler's quiet-timer, for the tests that do not look at it: never fires, nothing to clear.
const never = () => 0;
const noop = () => {};

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

    // A fake clock with frames every 16.7 ms; a job advances it by what it costs.
    const timedClock = () => {
        const c = clock();
        let t = 0;
        return {
            ...c,
            now: () => t,
            spend: (ms: number) => {
                t += ms;
            },
            frame: () => {
                t = (Math.floor(t / 16.7) + 1) * 16.7;
                c.frame();
            },
        };
    };

    it("still runs the per-frame limit when refits are cheap", () => {
        const c = timedClock();
        const s = createRefitScheduler(2, c.request, c.now);
        let ran = 0;
        const job = () => {
            ran++;
            c.spend(1);
        };
        const keys = Array.from({ length: 6 }, () => ({}));
        keys.forEach((k) => s.schedule(k, job));
        expect(ran).toBe(2);
        c.frame();
        expect(ran).toBe(4);
        c.frame();
        expect(ran).toBe(6);
    });

    it("runs at most one costly refit a frame, and about one every few frames", () => {
        const c = timedClock();
        const s = createRefitScheduler(2, c.request, c.now, never, noop);
        const perFrame: number[] = [];
        let ran = 0;
        const job = () => {
            ran++;
            c.spend(7.5);
        };
        const keys = Array.from({ length: 8 }, () => ({}));
        keys.forEach((k) => s.schedule(k, job));
        perFrame.push(ran);
        for (let i = 0; i < 80 && ran < keys.length; i++) {
            const before = ran;
            c.frame();
            perFrame.push(ran - before);
        }
        expect(Math.max(...perFrame)).toBe(1);
        expect(ran).toBe(keys.length);
        // 8 refits of 7.5 ms at 8% of the time take ~40 frames, not 4 (and not the ~14 of a 25% share).
        expect(perFrame.length).toBeGreaterThan(30);
        expect(perFrame.length).toBeLessThan(50);
    });

    it("runs every waiting refit at once when no refit has been asked for a while: a drag ends on the exact size", () => {
        const c = timedClock();
        let pending = null as { cb: () => void; ms: number } | null;
        const s = createRefitScheduler(
            2,
            c.request,
            c.now,
            (cb, ms) => (pending = { cb, ms }),
            () => {
                pending = null;
            }
        );
        const sizes = new Map<object, number>();
        const keys = Array.from({ length: 8 }, () => ({}));
        const refit = (k: object, size: number) => () => {
            sizes.set(k, size);
            c.spend(7.5);
        };
        // The drag's last size is 7; nobody is refitted at it yet except those the credit allowed.
        keys.forEach((k) => s.schedule(k, refit(k, 7)));
        expect(sizes.size).toBeLessThan(keys.length);
        expect(pending?.ms).toBe(LIVE_REFIT_QUIET_MS);
        pending?.cb();
        expect([...sizes.values()]).toEqual(keys.map(() => 7));
        expect(sizes.size).toBe(keys.length);
    });

    it("waits for the quiet again whenever another refit is asked for", () => {
        const c = timedClock();
        let timers = 0;
        let cleared = 0;
        const s = createRefitScheduler(
            2,
            c.request,
            c.now,
            () => ++timers,
            () => {
                cleared++;
            }
        );
        s.schedule({}, () => {});
        s.schedule({}, () => {});
        s.schedule({}, () => {});
        expect(timers).toBe(3);
        expect(cleared).toBe(2); // each new request cancels the timer before it
    });

    it("lets credit build up only to a limit while idle", () => {
        const c = timedClock();
        const s = createRefitScheduler(2, c.request, c.now);
        c.spend(10_000);
        let ran = 0;
        const job = () => {
            ran++;
            c.spend(7.5);
        };
        [{}, {}, {}].forEach((k) => s.schedule(k, job));
        expect(ran).toBe(1);
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
