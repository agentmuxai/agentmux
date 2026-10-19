// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { arrivalTotals, arrivalsBetween } from "./lib/wsarrivals.mjs";

describe("the page's time handling websocket messages", () => {
    // [bucket start, messages, bytes, biggest, processing ms]
    const ws = [[1000, 40, 4000, 300, 80], [1100, 50, 5000, 300, 95], [1200, 30, 3000, 200, 20]];

    it("adds up the processing time and gives it as a share of the window", () => {
        const w = arrivalsBetween(ws, 1000, 1300);
        expect(w.processingMs).toBe(195);
        expect(w.processingShare).toBe(0.65); // 195 ms of a 300 ms window: the page is busy with these messages most of the time
        expect(arrivalTotals(ws).processingMs).toBe(195);
    });

    it("treats an older record without the figure as zero", () => {
        const old = [[1000, 5, 500, 200]];
        expect(arrivalsBetween(old, 1000, 1100).processingMs).toBe(0);
    });
});
