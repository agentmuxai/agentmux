// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

import { FILL_HOLD_RECHECK_MS, planFill, runHistoryFill, type FillDeps } from "./history-fill";

describe("planFill", () => {
    const base = { startLine: 9_900, endLine: 10_000, sliceBytes: 64_000, windowLines: 10_000 };

    it("fills down to the restore window's start, within the feed's bytes", () => {
        expect(planFill({ ...base, endLine: 50_000, startLine: 49_900, restoreBytes: 1_000_000 })).toEqual({
            floorLine: 40_000,
            budgetBytes: 936_000,
        });
    });

    it("has no byte cap without a restore size: the line window alone bounds it", () => {
        expect(planFill(base)?.budgetBytes).toBe(Number.POSITIVE_INFINITY);
    });

    it("fills nothing when the first screen already reaches the window's start", () => {
        expect(planFill({ ...base, startLine: 0, endLine: 5 })).toBeNull();
        expect(planFill({ ...base, startLine: 40_000, endLine: 50_000 })).toBeNull();
    });

    it("fills nothing when the first screen already spent the byte budget", () => {
        expect(planFill({ ...base, endLine: 50_000, startLine: 40_001, sliceBytes: 1_000_000, restoreBytes: 1_000_000 })).toBeNull();
    });
});

/** A fill over a document whose earliest loaded line is `start`, with pages of `perLine` bytes a line. */
function harness(start: number, over: Partial<FillDeps> = {}, perLine = 100) {
    let offset = start;
    const pages: number[] = [];
    const deps: FillDeps = {
        plan: { floorLine: 0, budgetBytes: Number.POSITIVE_INFINITY },
        pageLines: 2_000,
        offset: () => offset,
        page: vi.fn(async (maxLines: number) => {
            pages.push(maxLines);
            offset -= maxLines;
            return { lines: maxLines, bytes: maxLines * perLine };
        }),
        stopped: () => false,
        holding: () => false,
        idle: async () => {},
        wait: async () => {},
        ...over,
    };
    /** What a real page does besides returning: the document's earliest line moves down. */
    const move = (n: number) => {
        offset -= n;
    };
    return { deps, pages, offset: () => offset, move };
}

describe("runHistoryFill", () => {
    it("pages down to the floor, the last page cut to what is left", async () => {
        const h = harness(5_000);
        const out = await runHistoryFill(h.deps);
        expect(h.pages).toEqual([2_000, 2_000, 1_000]);
        expect(h.offset()).toBe(0);
        expect(out).toEqual({ outcome: "done", pages: 3, lines: 5_000, bytes: 500_000 });
    });

    it("stops at the window's floor, not at the start of the transcript", async () => {
        const h = harness(30_000, { plan: { floorLine: 26_000, budgetBytes: Number.POSITIVE_INFINITY } });
        await runHistoryFill(h.deps);
        expect(h.pages).toEqual([2_000, 2_000]);
        expect(h.offset()).toBe(26_000);
    });

    it("stops once the byte budget is spent", async () => {
        const h = harness(30_000, { plan: { floorLine: 0, budgetBytes: 250_000 } });
        const out = await runHistoryFill(h.deps);
        // 200,000 bytes after one page, 400,000 after two: the second crosses it.
        expect(out.pages).toBe(2);
        expect(out.outcome).toBe("done");
    });

    it("stops after a page the byte budget cut, however much budget is left", async () => {
        const h = harness(30_000);
        h.deps.page = vi.fn(async () => {
            h.move(10);
            return { lines: 10, bytes: 10, last: true };
        });
        const out = await runHistoryFill(h.deps);
        expect(out.pages).toBe(1);
        expect(out.outcome).toBe("done");
    });

    it("waits while the user is active, then carries on", async () => {
        let held = 3;
        const wait = vi.fn(async () => {});
        const h = harness(2_000, { holding: () => held-- > 0, wait });
        const out = await runHistoryFill(h.deps);
        expect(wait).toHaveBeenCalledWith(FILL_HOLD_RECHECK_MS);
        expect(wait.mock.calls.length).toBeGreaterThanOrEqual(2);
        expect(out).toMatchObject({ outcome: "done", pages: 1 });
    });

    it("never reads while it is holding", async () => {
        let held = true;
        let checks = 0;
        const h = harness(2_000, {
            holding: () => held,
            wait: async () => {
                if (++checks === 5) held = false;
            },
        });
        const reads = h.deps.page as ReturnType<typeof vi.fn>;
        reads.mockImplementation(async (n: number) => {
            expect(held).toBe(false);
            h.move(n);
            return { lines: n, bytes: 1 };
        });
        await runHistoryFill(h.deps);
        expect(reads).toHaveBeenCalledTimes(1);
    });

    it("ends as stopped when the pane closes, between pages", async () => {
        let closed = false;
        const h = harness(10_000, { stopped: () => closed });
        (h.deps.page as ReturnType<typeof vi.fn>).mockImplementation(async (n: number) => {
            closed = true;
            h.move(n);
            return { lines: n, bytes: 1 };
        });
        const out = await runHistoryFill(h.deps);
        expect(out.outcome).toBe("stopped");
        expect(out.pages).toBe(1);
    });

    it("ends as failed when a page can't be read, leaving what loaded", async () => {
        let n = 0;
        const h = harness(10_000);
        h.deps.page = vi.fn(async (max: number) => {
            if (++n === 2) return null;
            h.move(max);
            return { lines: max, bytes: 1 };
        });
        const out = await runHistoryFill(h.deps);
        expect(out).toMatchObject({ outcome: "failed", pages: 1 });
    });

    it("ends as stopped, not failed, when a page returns nothing because the pane closed", async () => {
        let closed = false;
        const h = harness(10_000, {
            stopped: () => closed,
            page: vi.fn(async () => {
                closed = true;
                return null;
            }),
        });
        expect((await runHistoryFill(h.deps)).outcome).toBe("stopped");
    });

    it("doesn't repeat a page that moved nothing", async () => {
        const h = harness(10_000, { page: vi.fn(async () => ({ lines: 0, bytes: 0 })) });
        const out = await runHistoryFill(h.deps);
        expect(out.outcome).toBe("failed");
        expect(h.deps.page).toHaveBeenCalledTimes(1);
    });

    it("does nothing when there is nothing below the floor", async () => {
        const h = harness(0);
        const out = await runHistoryFill(h.deps);
        expect(out).toEqual({ outcome: "done", pages: 0, lines: 0, bytes: 0 });
        expect(h.deps.page).not.toHaveBeenCalled();
    });
});
