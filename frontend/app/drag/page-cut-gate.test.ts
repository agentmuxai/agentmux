// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { beginPageCut, PAGE_CUT_MAX_MS, pageCutsPending, resetPageCutGateForTests } from "./page-cut-gate";

beforeEach(() => {
    vi.useFakeTimers();
    resetPageCutGateForTests();
});
afterEach(() => vi.useRealTimers());

describe("the gate a browser page's cut holds the drag's reflow behind", () => {
    it("is held from the start of the cut until the cut reports, however many frames go by", () => {
        expect(pageCutsPending()).toBe(false);
        const cut = beginPageCut();
        expect(pageCutsPending()).toBe(true);
        // A hide the page's freeze-frame defers (up to 300 ms, pane-overlay.ts) is still not a cut.
        vi.advanceTimersByTime(300);
        expect(pageCutsPending()).toBe(true);
        cut();
        expect(pageCutsPending()).toBe(false);
    });

    it("stops holding by itself when a cut never reports", () => {
        beginPageCut();
        vi.advanceTimersByTime(PAGE_CUT_MAX_MS - 1);
        expect(pageCutsPending()).toBe(true);
        vi.advanceTimersByTime(1);
        expect(pageCutsPending()).toBe(false);
    });

    it("is held until every page's cut has reported", () => {
        const one = beginPageCut();
        const two = beginPageCut();
        one();
        expect(pageCutsPending()).toBe(true);
        two();
        expect(pageCutsPending()).toBe(false);
    });

    it("a cut that reports twice, or after its time ran out, counts once", () => {
        const one = beginPageCut();
        const two = beginPageCut();
        one();
        one();
        expect(pageCutsPending()).toBe(true);
        vi.advanceTimersByTime(PAGE_CUT_MAX_MS);
        expect(pageCutsPending()).toBe(false);
        two();
        const three = beginPageCut();
        expect(pageCutsPending()).toBe(true);
        three();
        expect(pageCutsPending()).toBe(false);
    });
});
