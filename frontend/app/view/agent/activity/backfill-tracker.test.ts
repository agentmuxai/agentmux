// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * backfill-tracker.ts — see docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md
 * and this repo's own docs/reports/REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md
 * Tier 1 recommendation.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));

import {
    createBackfillAwareTrigger,
    handleBackfillStatusEvent,
    holdBackfillingRows,
    isBlockBackfilling,
    onBackfillSettle,
    parseBackfillStatusEvent,
} from "./backfill-tracker";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("parseBackfillStatusEvent", () => {
    it("parses a valid started event", () => {
        expect(parseBackfillStatusEvent(["block:b1"], { status: "started" })).toEqual({
            blockId: "b1",
            status: "started",
        });
    });

    it("parses a valid done event", () => {
        expect(parseBackfillStatusEvent(["block:b1"], { status: "done" })).toEqual({
            blockId: "b1",
            status: "done",
        });
    });

    it("finds the block: scope among several unrelated scopes", () => {
        expect(parseBackfillStatusEvent(["other:x", "block:b2", "another:y"], { status: "done" })).toEqual({
            blockId: "b2",
            status: "done",
        });
    });

    it("returns null when there's no block: scope at all", () => {
        expect(parseBackfillStatusEvent(["other:x"], { status: "started" })).toBeNull();
        expect(parseBackfillStatusEvent(undefined, { status: "started" })).toBeNull();
        expect(parseBackfillStatusEvent([], { status: "started" })).toBeNull();
    });

    it("returns null for a malformed/unexpected status value", () => {
        expect(parseBackfillStatusEvent(["block:b1"], { status: "bogus" })).toBeNull();
        expect(parseBackfillStatusEvent(["block:b1"], {})).toBeNull();
        expect(parseBackfillStatusEvent(["block:b1"], null)).toBeNull();
        expect(parseBackfillStatusEvent(["block:b1"], undefined)).toBeNull();
    });
});

describe("isBlockBackfilling / handleBackfillStatusEvent", () => {
    it("is false for every block with nothing tracked", () => {
        expect(isBlockBackfilling("b1")).toBe(false);
    });

    it("becomes true on started, false again on done — for THAT block only", () => {
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        expect(isBlockBackfilling("b1")).toBe(true);
        expect(isBlockBackfilling("b2")).toBe(false);

        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(isBlockBackfilling("b1")).toBe(false);
    });

    it("tracks several concurrently-backfilling blocks independently", () => {
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        handleBackfillStatusEvent(["block:b2"], { status: "started" });

        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(isBlockBackfilling("b1")).toBe(false);
        expect(isBlockBackfilling("b2")).toBe(true); // b2 still in flight

        handleBackfillStatusEvent(["block:b2"], { status: "done" });
        expect(isBlockBackfilling("b2")).toBe(false);
    });

    it("a malformed event is silently ignored, not treated as a state transition", () => {
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        handleBackfillStatusEvent(["not-a-block-scope"], { status: "done" });
        expect(isBlockBackfilling("b1")).toBe(true); // b1 unaffected by the bogus event
        handleBackfillStatusEvent(["block:b1"], { status: "done" }); // cleanup for other tests
    });

    it("the safety-net timeout auto-clears a block whose done never arrives", () => {
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        vi.advanceTimersByTime(19_000);
        expect(isBlockBackfilling("b1")).toBe(true); // still under the 20s ceiling

        vi.advanceTimersByTime(2000); // total 21s — past it
        expect(isBlockBackfilling("b1")).toBe(false);
    });
});

describe("onBackfillSettle", () => {
    it("fires with the block id each time ONE block's backfill finishes, not only when all have", () => {
        const listener = vi.fn();
        const unsub = onBackfillSettle(listener);
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        handleBackfillStatusEvent(["block:b2"], { status: "started" });

        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(listener).toHaveBeenCalledTimes(1);
        expect(listener).toHaveBeenLastCalledWith("b1");

        handleBackfillStatusEvent(["block:b2"], { status: "done" });
        expect(listener).toHaveBeenCalledTimes(2);
        expect(listener).toHaveBeenLastCalledWith("b2");
        unsub();
    });

    it("fires for a block cleared by the safety-net timeout", () => {
        const listener = vi.fn();
        const unsub = onBackfillSettle(listener);
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        vi.advanceTimersByTime(21_000);
        expect(listener).toHaveBeenCalledWith("b1");
        unsub();
    });

    it("does not fire for a done with no matching started (nothing was being held)", () => {
        const listener = vi.fn();
        const unsub = onBackfillSettle(listener);
        handleBackfillStatusEvent(["block:never-started"], { status: "done" });
        expect(listener).not.toHaveBeenCalled();
        unsub();
    });

    it("an unsubscribed listener never fires", () => {
        const listener = vi.fn();
        onBackfillSettle(listener)();
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(listener).not.toHaveBeenCalled();
    });
});

describe("holdBackfillingRows", () => {
    const row = (id: string, parent: string, v = 0) => ({ id, parent_block_id: parent, v });

    it("returns the fresh list untouched when nothing is backfilling", () => {
        const next = [row("x", "b1", 1)];
        expect(holdBackfillingRows([row("x", "b1", 0)], next)).toBe(next);
    });

    it("keeps a backfilling pane's previous rows and takes every other pane's fresh rows", () => {
        handleBackfillStatusEvent(["block:A"], { status: "started" });
        const prev = [row("a1", "A", 0), row("b1", "B", 0)];
        // A's snapshot is still converging (a1 changed, a2 half-replayed);
        // B genuinely changed and must update NOW, not after A settles.
        const next = [row("a1", "A", 9), row("a2", "A", 9), row("b1", "B", 1), row("b2", "B", 1)];

        const merged = holdBackfillingRows(prev, next);

        expect(merged.filter((r) => r.parent_block_id === "A")).toEqual([row("a1", "A", 0)]);
        expect(merged.filter((r) => r.parent_block_id === "B")).toEqual([row("b1", "B", 1), row("b2", "B", 1)]);
        handleBackfillStatusEvent(["block:A"], { status: "done" });
    });

    it("once the pane settles, its fresh rows come through", () => {
        handleBackfillStatusEvent(["block:A"], { status: "started" });
        handleBackfillStatusEvent(["block:A"], { status: "done" });
        const next = [row("a1", "A", 9)];
        expect(holdBackfillingRows([row("a1", "A", 0)], next)).toBe(next);
    });
});

describe("createBackfillAwareTrigger", () => {
    it("delegates straight to the debounced scheduler when nothing is backfilling", () => {
        const scheduleDebounced = vi.fn();
        const refreshNow = vi.fn();
        const trigger = createBackfillAwareTrigger(scheduleDebounced, refreshNow);
        trigger();
        trigger();
        expect(scheduleDebounced).toHaveBeenCalledTimes(2);
        expect(refreshNow).not.toHaveBeenCalled();
    });

    it("keeps refreshing (debounced) while ANOTHER pane backfills — no app-wide freeze", () => {
        const scheduleDebounced = vi.fn();
        const trigger = createBackfillAwareTrigger(scheduleDebounced, vi.fn());

        handleBackfillStatusEvent(["block:A"], { status: "started" });
        trigger(); // e.g. pane B's subagent finished
        expect(scheduleDebounced).toHaveBeenCalledTimes(1);
        handleBackfillStatusEvent(["block:A"], { status: "done" });
    });

    it("fires ONE immediate refresh when a pane that saw events mid-backfill settles", () => {
        const refreshNow = vi.fn();
        const trigger = createBackfillAwareTrigger(vi.fn(), refreshNow);

        handleBackfillStatusEvent(["block:A"], { status: "started" });
        for (let i = 0; i < 50; i++) trigger();
        expect(refreshNow).not.toHaveBeenCalled();

        handleBackfillStatusEvent(["block:A"], { status: "done" });
        expect(refreshNow).toHaveBeenCalledTimes(1); // not 50
    });

    it("settles each pane on its own — the first to finish doesn't wait for the others", () => {
        const refreshNow = vi.fn();
        const trigger = createBackfillAwareTrigger(vi.fn(), refreshNow);

        handleBackfillStatusEvent(["block:A"], { status: "started" });
        handleBackfillStatusEvent(["block:B"], { status: "started" });
        trigger();

        handleBackfillStatusEvent(["block:A"], { status: "done" });
        expect(refreshNow).toHaveBeenCalledTimes(1); // A's rows land now, B still backfilling

        handleBackfillStatusEvent(["block:B"], { status: "done" });
        expect(refreshNow).toHaveBeenCalledTimes(2);
    });

    it("no settle refresh for a backfill that saw no events", () => {
        const refreshNow = vi.fn();
        createBackfillAwareTrigger(vi.fn(), refreshNow);
        handleBackfillStatusEvent(["block:A"], { status: "started" });
        handleBackfillStatusEvent(["block:A"], { status: "done" });
        expect(refreshNow).not.toHaveBeenCalled();
    });
});

describe("the live MPS subscription wired at module load", () => {
    it("registered a handler for subagent:backfill_status", () => {
        expect(hub.handlers.has("subagent:backfill_status")).toBe(true);
    });

    it("routes a real event through to state, exactly like calling handleBackfillStatusEvent directly", () => {
        const handler = hub.handlers.get("subagent:backfill_status")!;
        handler({ scopes: ["block:live-test"], data: { status: "started" } });
        expect(isBlockBackfilling("live-test")).toBe(true);
        handler({ scopes: ["block:live-test"], data: { status: "done" } });
        expect(isBlockBackfilling("live-test")).toBe(false);
    });
});
