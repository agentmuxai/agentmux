// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { setLastInputForTests } from "@/app/store/last-input";
import { registerRecycleBlocker, reasonWithHold, recycleBlockerHolds } from "@/app/store/recycle-blockers";
import { collectPaneStats, paneTotals, registerPaneStats, searchState } from "./pane-stats";
import { collectRendererState, logRendererState, resetRendererStateForTests } from "./renderer-state-diagnostic";

const MB = 1024 * 1024;
const MIN = 60_000;

/** Pretend the engine reports this JS heap (Chromium's non-standard performance.memory). */
function heapIs(usedMb: number): void {
    Object.defineProperty(performance, "memory", {
        configurable: true,
        value: { usedJSHeapSize: usedMb * MB, totalJSHeapSize: (usedMb + 20) * MB, jsHeapSizeLimit: 4096 * MB },
    });
}

afterEach(() => {
    delete (performance as unknown as { memory?: unknown }).memory;
    resetRendererStateForTests();
    vi.restoreAllMocks();
});

describe("pane stats", () => {
    it("lists mounted panes largest first, and forgets one that unmounts", () => {
        const offA = registerPaneStats("aaaaaaaa-1111", () => ({ rows: 40, search: 0 }));
        const offB = registerPaneStats("bbbbbbbb-2222", () => ({ rows: 900, search: 2 }));
        expect(collectPaneStats().map((s) => s.blockId)).toEqual(["bbbbbbbb-2222", "aaaaaaaa-1111"]);
        expect(paneTotals()).toEqual({ panes: 2, rows: 940, searches: 1, maxRows: 900 });
        offB();
        expect(collectPaneStats().map((s) => s.blockId)).toEqual(["aaaaaaaa-1111"]);
        offA();
        expect(collectPaneStats()).toEqual([]);
    });

    it("a pane that throws while tearing down does not cost the others", () => {
        const offBad = registerPaneStats("bad", () => {
            throw new Error("gone");
        });
        const offOk = registerPaneStats("ok", () => ({ rows: 3, search: 0 }));
        expect(collectPaneStats().map((s) => s.blockId)).toEqual(["ok"]);
        offBad();
        offOk();
    });

    it("tells a closed bar, an empty open bar and a query apart", () => {
        expect(searchState(false, "abc")).toBe(0);
        expect(searchState(true, undefined)).toBe(1);
        expect(searchState(true, "")).toBe(1);
        expect(searchState(true, "abc")).toBe(2);
    });
});

describe("the renderer state line", () => {
    it("reports the heap, the panes and what holds a recycle, with no text", () => {
        heapIs(900);
        setLastInputForTests(25 * MIN);
        const off = [
            registerPaneStats("cccccccc-3333-4444", () => ({ rows: 1200, search: 2 })),
            registerRecycleBlocker(() => "a message is waiting to be sent"),
        ];
        const state = collectRendererState(Date.now());
        expect(state).toMatchObject({
            w: "main",
            idleMin: 25,
            heap: { used: 900, total: 920, limit: 4096 },
            drop: null,
            panes: { n: 1, rows: 1200, maxRows: 1200, searches: 1 },
            top: [{ b: "cccccccc", rows: 1200, s: 2 }],
            blocked: [{ why: "a message is waiting to be sent", heldMin: 0 }],
        });
        expect(state.dom).toBeGreaterThan(0);
        off.forEach((f) => f());
    });

    it("notes when the heap last fell by a collection's worth, and ignores small dips", () => {
        const t0 = Date.now();
        heapIs(1000);
        collectRendererState(t0);
        heapIs(980); // a 2% dip is not a collection
        expect(collectRendererState(t0 + 5 * MIN).drop).toBeNull();
        heapIs(300);
        expect(collectRendererState(t0 + 10 * MIN).drop).toEqual({ mb: 680, agoMin: 0 });
        heapIs(320);
        expect(collectRendererState(t0 + 25 * MIN).drop).toEqual({ mb: 680, agoMin: 15 });
    });

    it("says null for the heap where the engine does not report it", () => {
        expect(collectRendererState().heap).toBeNull();
    });

    it("is logged with console.log, which the host's default filter keeps", () => {
        const debug = vi.spyOn(console, "debug").mockImplementation(() => {});
        const log = vi.spyOn(console, "log").mockImplementation(() => {});
        logRendererState();
        expect(debug).not.toHaveBeenCalled();
        const [tag, payload] = log.mock.calls[0];
        expect(tag).toBe("[renderer-state]");
        expect(() => JSON.parse(payload as string)).not.toThrow();
    });
});

describe("how long a recycle blocker has held", () => {
    it("counts from the first look, and starts again after it lets go", () => {
        let blocking = true;
        const off = registerRecycleBlocker(() => (blocking ? "an upload is still going" : null));
        const t0 = Date.now();
        expect(recycleBlockerHolds(t0)).toEqual([{ reason: "an upload is still going", heldMs: 0 }]);
        expect(recycleBlockerHolds(t0 + 3 * 60 * MIN + 10 * MIN)[0].heldMs).toBe(190 * MIN);
        expect(reasonWithHold("an upload is still going", t0 + 190 * MIN)).toBe("an upload is still going (held 3h 10m)");
        expect(reasonWithHold("an upload is still going", t0 + 30 * 1000)).toBe("an upload is still going");
        blocking = false;
        expect(recycleBlockerHolds(t0 + 200 * MIN)).toEqual([]);
        blocking = true;
        expect(recycleBlockerHolds(t0 + 210 * MIN)[0].heldMs).toBe(0);
        off();
    });
});
