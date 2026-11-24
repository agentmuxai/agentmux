// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { SETTLE_MAX_MS, settledPaneRect } from "./settled-pane-rect";

const rect = (width: number, height = 800, x = 0, y = 0) => ({ x, y, width, height });

/** A page clock the test drives: frames run when `tick` is called, the timer when `fire` is. */
function clock() {
    let frameCb: (() => void) | null = null;
    let timerCb: (() => void) | null = null;
    let timerMs = -1;
    return {
        frame: (cb: () => void) => {
            frameCb = cb;
            return 1;
        },
        cancelFrame: () => {
            frameCb = null;
        },
        timer: (cb: () => void, ms: number) => {
            timerCb = cb;
            timerMs = ms;
            return 2;
        },
        cancelTimer: () => {
            timerCb = null;
        },
        tick() {
            const cb = frameCb;
            frameCb = null;
            cb?.();
        },
        fire() {
            const cb = timerCb;
            timerCb = null;
            cb?.();
        },
        get waitingForFrame() {
            return frameCb != null;
        },
        get timerMs() {
            return timerMs;
        },
    };
}

describe("settledPaneRect", () => {
    it("makes the page at the rect two frames agree on, not the narrow one it was first measured at", async () => {
        const c = clock();
        const sizes = [rect(686), rect(1200), rect(1586), rect(1586)];
        let i = 0;
        const measure = () => sizes[Math.min(i++, sizes.length - 1)];
        const got = settledPaneRect(measure, c);
        c.tick(); // 1200 vs 686
        c.tick(); // 1586 vs 1200
        c.tick(); // 1586 vs 1586
        expect(await got).toEqual(rect(1586));
        expect(c.waitingForFrame).toBe(false);
    });

    it("takes a pane that is already still after one frame", async () => {
        const c = clock();
        const got = settledPaneRect(() => rect(1586), c);
        c.tick();
        expect(await got).toEqual(rect(1586));
    });

    it("does not settle on an empty rect: a pane not laid out yet is waited for", async () => {
        const c = clock();
        let w = 0;
        const got = settledPaneRect(() => rect(w), c);
        c.tick();
        c.tick();
        expect(c.waitingForFrame).toBe(true);
        w = 900;
        c.tick();
        c.tick();
        expect(await got).toEqual(rect(900));
    });

    it("a moving pane moves the page's position too: x and y count", async () => {
        const c = clock();
        const seq = [rect(1000, 800, 0, 0), rect(1000, 800, 300, 0), rect(1000, 800, 300, 0)];
        let i = 0;
        const got = settledPaneRect(() => seq[Math.min(i++, 2)], c);
        c.tick();
        c.tick();
        expect(await got).toEqual(rect(1000, 800, 300, 0));
    });

    it("gives up at the time limit with the latest measurement when it never holds still", async () => {
        const c = clock();
        let w = 600;
        const got = settledPaneRect(() => rect((w += 10)), c);
        c.tick();
        c.tick();
        c.fire();
        const r = await got;
        expect(r.width).toBeGreaterThan(600);
        expect(c.waitingForFrame).toBe(false);
        expect(c.timerMs).toBe(SETTLE_MAX_MS);
    });

    it("still makes the page when no frame ever comes (a hidden window)", async () => {
        const c = clock();
        const got = settledPaneRect(() => rect(1586), c);
        c.fire();
        expect(await got).toEqual(rect(1586));
    });

    it("stops asking for frames and drops its timer once settled", async () => {
        const c = clock();
        const got = settledPaneRect(() => rect(1000), c);
        c.tick();
        await got;
        c.fire(); // cancelled: nothing happens
        expect(c.waitingForFrame).toBe(false);
    });
});
