// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    createNodeSettler,
    SETTLE_NODE_MAX_MS,
    SETTLE_NODE_QUIET_FRAMES,
    type NodeSettlerHost,
} from "./settle-on-node";

/** A page of 16 ms frames, a clock that moves with them, and a position the settler writes. */
function page(aims: (frame: number) => number | null, ready: (frame: number) => boolean = () => true) {
    let clock = 0;
    let frames = 0;
    let pending: (() => void) | null = null;
    const writes: number[] = [];
    const host = {
        user: false,
        key: "100%",
        cancelled: 0,
    };
    const h: NodeSettlerHost = {
        aim: () => aims(frames),
        write: (top) => writes.push(top),
        userActive: () => host.user,
        viewKey: () => host.key,
        ready: () => ready(frames),
        frame: (cb) => {
            pending = cb;
            return 1;
        },
        cancelFrame: () => {
            host.cancelled++;
            pending = null;
        },
        now: () => clock,
    };
    const step = (): boolean => {
        const cb = pending;
        if (!cb) return false;
        pending = null;
        clock += 16;
        frames++;
        cb();
        return true;
    };
    const run = (max = 200): number => {
        let n = 0;
        while (n < max && step()) n++;
        return n;
    };
    return { h, host, writes, step, run, frames: () => frames, now: () => clock };
}

describe("createNodeSettler", () => {
    it("writes the first aim at once, with no smooth scroll", () => {
        const p = page(() => 1000);
        createNodeSettler(p.h).start();
        expect(p.writes).toEqual([1000]);
    });

    it("lands when the aim holds for the quiet frames, writing nothing more", () => {
        const p = page(() => 1000);
        const s = createNodeSettler(p.h);
        s.start();
        const frames = p.run();
        expect(frames).toBe(SETTLE_NODE_QUIET_FRAMES);
        expect(p.writes).toEqual([1000]);
        expect(s.active()).toBe(false);
    });

    it("re-aims when a measurement moves the target, and counts the quiet frames again", () => {
        // Estimated at 1000; measured rows push it to 1400 on frame 2 and to 1460 on frame 4.
        const p = page((f) => (f < 2 ? 1000 : f < 4 ? 1400 : 1460));
        const s = createNodeSettler(p.h);
        s.start();
        p.run();
        expect(p.writes).toEqual([1000, 1400, 1460]);
        expect(s.active()).toBe(false);
        expect(p.frames()).toBe(4 + SETTLE_NODE_QUIET_FRAMES);
    });

    it("does not take a held aim as landed while the layout has not caught up, however many frames pass", () => {
        // The window follows the scroll position late: the aim holds, but the rows are not measured until frame 9.
        const p = page(
            () => 1000,
            (f) => f >= 9
        );
        const s = createNodeSettler(p.h);
        s.start();
        p.run();
        expect(p.frames()).toBe(9 + SETTLE_NODE_QUIET_FRAMES - 1);
        expect(p.writes).toEqual([1000]);
        expect(s.active()).toBe(false);
    });

    it("re-aims while not ready, and the time limit still ends a layout that never gets ready", () => {
        const p = page(
            (f) => 1000 + (f < 4 ? 0 : 300),
            () => false
        );
        const s = createNodeSettler(p.h);
        s.start();
        p.run(500);
        expect(p.writes).toEqual([1000, 1300]);
        expect(p.now()).toBeGreaterThanOrEqual(SETTLE_NODE_MAX_MS);
        expect(s.active()).toBe(false);
    });

    it("ignores a change under a pixel", () => {
        const p = page((f) => 1000 + (f % 2) * 0.4);
        createNodeSettler(p.h).start();
        p.run();
        expect(p.writes).toEqual([1000]);
    });

    it("waits while the row has no place yet, then aims when it gets one", () => {
        const p = page((f) => (f < 3 ? null : 800));
        const s = createNodeSettler(p.h);
        s.start();
        expect(p.writes).toEqual([]);
        p.run();
        expect(p.writes).toEqual([800]);
        expect(s.active()).toBe(false);
    });

    it("stops at the time limit when the aim never settles", () => {
        const p = page((f) => 1000 + f * 50);
        const s = createNodeSettler(p.h);
        s.start();
        p.run(500);
        expect(s.active()).toBe(false);
        expect(p.now()).toBeGreaterThanOrEqual(SETTLE_NODE_MAX_MS);
        expect(p.now()).toBeLessThan(SETTLE_NODE_MAX_MS + 40);
    });

    it("stops at once when the person starts to scroll, and writes nothing after", () => {
        const p = page((f) => 1000 + f * 50);
        const s = createNodeSettler(p.h);
        s.start();
        p.step();
        p.host.user = true;
        const before = p.writes.length;
        p.run();
        expect(p.writes.length).toBe(before);
        expect(s.active()).toBe(false);
    });

    it("lets input that was already open when the jump began be", () => {
        const p = page(() => 1000);
        p.host.user = true;
        const s = createNodeSettler(p.h);
        s.start();
        p.run();
        expect(p.frames()).toBe(SETTLE_NODE_QUIET_FRAMES);
    });

    it("a newer jump replaces the one in progress", () => {
        let target = 1000;
        const p = page(() => target);
        const s = createNodeSettler(p.h);
        s.start();
        p.step();
        target = 5000;
        s.start();
        p.run();
        expect(p.writes).toEqual([1000, 5000]);
    });

    it("cancel stops it and drops the pending frame", () => {
        const p = page((f) => 1000 + f * 50);
        const s = createNodeSettler(p.h);
        s.start();
        s.cancel();
        expect(p.run()).toBe(0);
        expect(s.active()).toBe(false);
    });

    it("a zoom change or a resize during the jump ends it without moving the position", () => {
        const p = page((f) => 1000 + f * 50);
        const s = createNodeSettler(p.h);
        s.start();
        p.step();
        p.host.key = "150%";
        const before = p.writes.length;
        p.run();
        expect(p.writes.length).toBe(before);
        expect(s.active()).toBe(false);
    });
});
