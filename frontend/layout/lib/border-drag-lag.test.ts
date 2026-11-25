// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { beginBorderDrag, endBorderDrag, LATE_LAG_MS, summarizeLags } from "./border-drag-lag";

describe("summarizeLags", () => {
    it("is the median, the 95th percentile, the worst and the count over the late line", () => {
        const lags = Array.from({ length: 20 }, (_, i) => (i === 19 ? 120 : 10 + i));
        const s = summarizeLags(40, lags);
        expect(s).toEqual({ moves: 40, drawn: 20, p50: 20, p95: 120, worst: 120, late: 1 });
    });
    it("is zeros for a drag no frame drew", () => {
        expect(summarizeLags(3, [])).toEqual({ moves: 3, drawn: 0, p50: 0, p95: 0, worst: 0, late: 0 });
    });
});

describe("a border drag's line", () => {
    let frameDelay = 16;
    beforeEach(() => {
        vi.useFakeTimers();
        frameDelay = 16;
        vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => window.setTimeout(() => cb(performance.now()), frameDelay) as unknown as number);
    });
    afterEach(() => {
        endBorderDrag(() => {});
        vi.restoreAllMocks();
        vi.useRealTimers();
    });

    // The input's own time stamp, on the (fake) clock the frames run on.
    const move = () => {
        const e = new Event("pointermove");
        Object.defineProperty(e, "timeStamp", { value: performance.now() });
        window.dispatchEvent(e);
    };

    it("says how far the border lagged the pointer, once, at the release", () => {
        const lines: string[] = [];
        beginBorderDrag();
        for (let i = 0; i < 10; i++) {
            move();
            vi.advanceTimersByTime(16);
        }
        endBorderDrag((l) => lines.push(l));
        expect(lines).toHaveLength(1);
        expect(lines[0]).toMatch(/^\[perf\] border-drag moves=10 frames-drawn=\d+ lag p50=\d+ms p95=\d+ms worst=\d+ms late\(>50ms\)=0$/);
    });

    it("counts a frame that was drawn late as late", () => {
        const lines: string[] = [];
        frameDelay = LATE_LAG_MS + 70;
        beginBorderDrag();
        for (let i = 0; i < 6; i++) {
            move();
            vi.advanceTimersByTime(frameDelay + 5);
        }
        endBorderDrag((l) => lines.push(l));
        expect(lines[0]).toMatch(/late\(>50ms\)=6$/);
    });

    it("says nothing for a press on the border that did not move", () => {
        const lines: string[] = [];
        beginBorderDrag();
        move();
        endBorderDrag((l) => lines.push(l));
        expect(lines).toEqual([]);
    });

    it("stops listening at the release", () => {
        const lines: string[] = [];
        beginBorderDrag();
        endBorderDrag(() => {});
        for (let i = 0; i < 10; i++) move();
        beginBorderDrag();
        endBorderDrag((l) => lines.push(l));
        expect(lines).toEqual([]);
    });
});
