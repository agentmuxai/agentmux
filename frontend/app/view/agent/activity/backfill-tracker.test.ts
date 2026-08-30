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

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));

import {
    createBackfillAwareTrigger,
    handleBackfillStatusEvent,
    isAnyBlockBackfilling,
    onNextBackfillSettle,
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

describe("isAnyBlockBackfilling / handleBackfillStatusEvent", () => {
    it("is false with nothing tracked", () => {
        expect(isAnyBlockBackfilling()).toBe(false);
    });

    it("becomes true on started, false again on done for the same block", () => {
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        expect(isAnyBlockBackfilling()).toBe(true);

        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(isAnyBlockBackfilling()).toBe(false);
    });

    it("stays true while ANY of several concurrently-backfilling blocks hasn't finished", () => {
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        handleBackfillStatusEvent(["block:b2"], { status: "started" });
        expect(isAnyBlockBackfilling()).toBe(true);

        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(isAnyBlockBackfilling()).toBe(true); // b2 still in flight

        handleBackfillStatusEvent(["block:b2"], { status: "done" });
        expect(isAnyBlockBackfilling()).toBe(false);
    });

    it("a malformed event is silently ignored, not treated as a state transition", () => {
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        handleBackfillStatusEvent(["not-a-block-scope"], { status: "done" });
        expect(isAnyBlockBackfilling()).toBe(true); // b1 unaffected by the bogus event
        handleBackfillStatusEvent(["block:b1"], { status: "done" }); // cleanup for other tests
    });

    it("the safety-net timeout auto-clears a block whose done never arrives", () => {
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        expect(isAnyBlockBackfilling()).toBe(true);

        vi.advanceTimersByTime(19_000);
        expect(isAnyBlockBackfilling()).toBe(true); // still under the 20s ceiling

        vi.advanceTimersByTime(2000); // total 21s — past it
        expect(isAnyBlockBackfilling()).toBe(false);
    });
});

describe("onNextBackfillSettle", () => {
    it("fires once, the next time the tracked set becomes empty", () => {
        const listener = vi.fn();
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        onNextBackfillSettle(listener);

        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(listener).toHaveBeenCalledTimes(1);

        // A later, unrelated settle must NOT re-fire the same (already
        // consumed) listener.
        handleBackfillStatusEvent(["block:b2"], { status: "started" });
        handleBackfillStatusEvent(["block:b2"], { status: "done" });
        expect(listener).toHaveBeenCalledTimes(1);
    });

    it("does not fire until EVERY tracked block has settled", () => {
        const listener = vi.fn();
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        handleBackfillStatusEvent(["block:b2"], { status: "started" });
        onNextBackfillSettle(listener);

        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(listener).not.toHaveBeenCalled();

        handleBackfillStatusEvent(["block:b2"], { status: "done" });
        expect(listener).toHaveBeenCalledTimes(1);
    });

    it("an unsubscribed listener never fires", () => {
        const listener = vi.fn();
        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        const unsub = onNextBackfillSettle(listener);
        unsub();

        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(listener).not.toHaveBeenCalled();
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

    it("suppresses the debounced scheduler entirely while a backfill is in flight", () => {
        const scheduleDebounced = vi.fn();
        const refreshNow = vi.fn();
        const trigger = createBackfillAwareTrigger(scheduleDebounced, refreshNow);

        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        for (let i = 0; i < 50; i++) trigger();
        expect(scheduleDebounced).not.toHaveBeenCalled();
        expect(refreshNow).not.toHaveBeenCalled();

        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(refreshNow).toHaveBeenCalledTimes(1);
        expect(scheduleDebounced).not.toHaveBeenCalled();
    });

    it("only registers ONE settle listener no matter how many events arrive mid-backfill", () => {
        const scheduleDebounced = vi.fn();
        const refreshNow = vi.fn();
        const trigger = createBackfillAwareTrigger(scheduleDebounced, refreshNow);

        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        trigger();
        trigger();
        trigger();
        handleBackfillStatusEvent(["block:b1"], { status: "done" });

        expect(refreshNow).toHaveBeenCalledTimes(1); // not 3
    });

    it("resumes normal debounced behavior for events arriving after a backfill settles", () => {
        const scheduleDebounced = vi.fn();
        const refreshNow = vi.fn();
        const trigger = createBackfillAwareTrigger(scheduleDebounced, refreshNow);

        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        trigger();
        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(refreshNow).toHaveBeenCalledTimes(1);

        trigger(); // a genuinely live event, well after settle
        expect(scheduleDebounced).toHaveBeenCalledTimes(1);
    });

    it("a second backfill window after the first settled is handled independently", () => {
        const scheduleDebounced = vi.fn();
        const refreshNow = vi.fn();
        const trigger = createBackfillAwareTrigger(scheduleDebounced, refreshNow);

        handleBackfillStatusEvent(["block:b1"], { status: "started" });
        trigger();
        handleBackfillStatusEvent(["block:b1"], { status: "done" });
        expect(refreshNow).toHaveBeenCalledTimes(1);

        handleBackfillStatusEvent(["block:b2"], { status: "started" });
        trigger();
        expect(refreshNow).toHaveBeenCalledTimes(1); // still 1 — suppressed again
        handleBackfillStatusEvent(["block:b2"], { status: "done" });
        expect(refreshNow).toHaveBeenCalledTimes(2);
    });
});

describe("the live WPS subscription wired at module load", () => {
    it("registered a handler for subagent:backfill_status", () => {
        expect(hub.handlers.has("subagent:backfill_status")).toBe(true);
    });

    it("routes a real event through to state, exactly like calling handleBackfillStatusEvent directly", () => {
        const handler = hub.handlers.get("subagent:backfill_status")!;
        handler({ scopes: ["block:live-test"], data: { status: "started" } });
        expect(isAnyBlockBackfilling()).toBe(true);
        handler({ scopes: ["block:live-test"], data: { status: "done" } });
        expect(isAnyBlockBackfilling()).toBe(false);
    });
});
