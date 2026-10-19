// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { pageLagIn, pingsIn, summariseWorkerProbe, workerProbeLine } from "./lib/wsworker.mjs";

// 100 messages per 100 ms from t = 10_000 for one second.
const stream = (delayMs, extra = 0) => Array.from({ length: 10 }, (_, i) => [10_000 + i * 100 + delayMs, 100 + extra, 1000, 10, 5]);

describe("how far behind the page's stream is", () => {
    it("is the delay between the same messages reaching the worker and the page", () => {
        const lag = pageLagIn(stream(500), stream(0), 10_000, 12_000);
        expect(lag).toBe(500);
    });

    it("is zero for two streams that arrive together", () => {
        expect(pageLagIn(stream(0), stream(0), 10_000, 12_000)).toBe(0);
    });

    it("looks only at the window it is given", () => {
        // the page was late only in its last buckets
        const page = stream(0).map((b, i) => (i >= 8 ? [b[0] + 700, ...b.slice(1)] : b));
        expect(pageLagIn(page, stream(0), 10_000, 10_700)).toBe(0);
        expect(pageLagIn(page, stream(0), 10_000, 12_000)).toBeGreaterThan(0);
    });

    it("keeps a lag that was there before the window started (the offset is taken while both were caught up)", () => {
        // the page is 500 ms behind from the start; the send's window is the last 500 ms
        expect(pageLagIn(stream(500), stream(0), 11_000, 12_000, 10_000)).toBe(500);
        // taken at the window's start (the old way) the same data would read as no lag
        expect(pageLagIn(stream(500), stream(0), 11_000, 12_000, 11_000)).toBeLessThan(500);
    });

    it("says it cannot line up a stream that is missing", () => {
        expect(pageLagIn(stream(0), [], 10_000, 12_000)).toBeNull();
        expect(pageLagIn(null, stream(0), 10_000, 12_000)).toBeNull();
    });
});

describe("the worker's own RPC", () => {
    const pings = [[10_000, 4], [10_250, 6], [10_500, null], [11_000, 900], [13_000, 3]];

    it("counts the pings sent and answered in a window, and the slowest answer", () => {
        expect(pingsIn(pings, 10_000, 11_000)).toEqual({ sent: 4, answered: 3, maxMs: 900 });
        expect(pingsIn(pings, 20_000, 21_000)).toEqual({ sent: 0, answered: 0, maxMs: null });
    });

    it("summarises a run: totals, latency, and each slow send against both", () => {
        const worker = { total: [1000, 2097152], buckets: stream(0).map((b) => [b[0], b[1], b[2]]), pings, unanswered: 1 };
        const p = summariseWorkerProbe(worker, stream(500), [{ round: 3, agent: 7, from: 10_000, to: 12_000 }]);
        expect(p.streams).toEqual({ pageMessages: 1000, workerMessages: 1000, workerMB: 2 });
        expect(p.workerRpc).toMatchObject({ sent: 5, unanswered: 1, maxMs: 900 });
        expect(p.slowSends[0]).toMatchObject({ round: 3, agent: 7, pageLagBehindWorkerMs: 500 });
        expect(p.slowSends[0].workerRpcInWindow.maxMs).toBe(900);
        expect(workerProbeLine(p)).toContain("up to 500 ms behind the worker's");
        expect(summariseWorkerProbe(null, [], [])).toBeNull();
    });

    it("counts only what the page received while the worker was listening", () => {
        const worker = { opened: 10_000, readAt: 10_950, total: [1000, 0], buckets: stream(0).map((b) => [b[0], b[1], b[2]]), pings: [], unanswered: 0 };
        const page = [...stream(0), [20_000, 500, 1, 1, 1]]; // 500 more messages long after the worker was stopped
        expect(summariseWorkerProbe(worker, page, []).streams.pageMessages).toBe(1000);
    });
});
