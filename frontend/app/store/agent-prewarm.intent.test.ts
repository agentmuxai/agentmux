// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The draw on intent: the panes of the tab the pointer rests on that hold something older than they have now draw,
 * one at a time, ahead of the switch (docs/specs/SPEC_WINDOW_TAB_SWITCH_DRAW_ON_INTENT_2026_11_12.md §4).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    countDrawnAhead,
    createPrewarmScheduler,
    PREWARM_LEASE_MAX_MS,
    takeDrawnAhead,
    type Pressure,
    type PrewarmCandidate,
} from "./agent-prewarm";

type Pane = PrewarmCandidate & { stale: () => boolean; drawable: () => boolean; isStale: boolean; isDrawable: boolean; nodes: number };

function setup(over: { enabled?: boolean; idle?: boolean; intentEnabled?: boolean; intentIdle?: boolean; pressure?: Pressure } = {}) {
    const state = {
        enabled: over.enabled ?? false,
        idle: over.idle ?? true,
        intentEnabled: over.intentEnabled ?? true,
        intentIdle: over.intentIdle ?? true,
        pressure: over.pressure ?? ("normal" as Pressure),
    };
    const settles: Array<() => void> = [];
    const lease = vi.fn();
    const log = vi.fn();
    const cancelSettle = vi.fn();
    const s = createPrewarmScheduler({
        enabled: () => state.enabled,
        idle: () => state.idle,
        intentEnabled: () => state.intentEnabled,
        intentIdle: () => state.intentIdle,
        pressure: async () => state.pressure,
        heapBytes: () => null,
        lease,
        settle: (done) => {
            settles.push(done);
            return cancelSettle;
        },
        log,
        now: () => 0,
    });
    const pane = (blockId: string, tabId: string): Pane => {
        const c: Pane = {
            blockId,
            tabId,
            isStale: true,
            isDrawable: true,
            nodes: 10,
            tabSeen: () => false,
            nodeCount: () => c.nodes,
            stale: () => c.isStale,
            drawable: () => c.isDrawable,
        };
        return c;
    };
    /** The newest pane's draw ends: its lease is given back and the scheduler moves on. */
    const settle = async () => {
        settles.pop()?.();
        await vi.advanceTimersByTimeAsync(0);
    };
    return { s, state, lease, log, cancelSettle, pane, settle, settles };
}

describe("draw on intent", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        vi.useRealTimers();
        // The note of what was drawn ahead is module-wide: clear it between tests.
        for (const id of ["a1", "a2", "b1", "idle1", "fresh", "behind", "other"]) takeDrawnAhead(id);
    });

    it("draws the stale panes of the tab one at a time, and gives each lease back when it has settled", async () => {
        const { s, lease, pane, settle } = setup();
        s.add(pane("a1", "t1"));
        s.add(pane("a2", "t1"));
        s.add(pane("b1", "t2"));
        s.intent("t1");
        await vi.advanceTimersByTimeAsync(0);
        expect(lease).toHaveBeenLastCalledWith("a1");
        await settle();
        expect(lease).toHaveBeenCalledWith(null);
        expect(lease).toHaveBeenLastCalledWith("a2");
        await settle();
        expect(lease).toHaveBeenLastCalledWith(null);
        expect(lease.mock.calls.map((c) => c[0])).toEqual(["a1", null, "a2", null]);
        expect(countDrawnAhead(["a1", "a2", "b1"])).toBe(2);
    });

    it("leaves out a pane that holds what it has now, one that cannot draw while hidden, and other tabs", async () => {
        const { s, lease, pane } = setup();
        const fresh = pane("fresh", "t1");
        fresh.isStale = false;
        const behind = pane("behind", "t1");
        behind.isDrawable = false;
        s.add(fresh);
        s.add(behind);
        s.add(pane("other", "t2"));
        s.intent("t1");
        await vi.advanceTimersByTimeAsync(0);
        expect(lease).not.toHaveBeenCalled();
    });

    it("starts no pane while the window is not idle, with the setting off or under memory pressure", async () => {
        for (const over of [{ intentIdle: false }, { intentEnabled: false }, { pressure: "warn" as Pressure }]) {
            const { s, lease, pane } = setup(over);
            s.add(pane("a1", "t1"));
            s.intent("t1");
            await vi.advanceTimersByTimeAsync(0);
            expect(lease).not.toHaveBeenCalled();
        }
    });

    it("lets the pane that is drawing finish when the pointer leaves, and starts no other", async () => {
        const { s, lease, pane, settle } = setup();
        s.add(pane("a1", "t1"));
        s.add(pane("a2", "t1"));
        s.intent("t1");
        await vi.advanceTimersByTimeAsync(0);
        s.intent(null);
        expect(lease).toHaveBeenCalledTimes(1); // a1 still holds it: a half-drawn pane would show as one
        await settle();
        expect(lease.mock.calls.map((c) => c[0])).toEqual(["a1", null]);
    });

    it("moves on to the tab the pointer went to while a pane was drawing", async () => {
        const { s, lease, pane, settle } = setup();
        s.add(pane("a1", "t1"));
        s.add(pane("b1", "t2"));
        s.intent("t1");
        await vi.advanceTimersByTimeAsync(0);
        s.intent("t2");
        await settle();
        expect(lease.mock.calls.map((c) => c[0])).toEqual(["a1", null, "b1"]);
    });

    it("ends a draw that has not settled at the lease cap, and says so", async () => {
        const { s, lease, log, pane } = setup();
        s.add(pane("a1", "t1"));
        s.intent("t1");
        await vi.advanceTimersByTimeAsync(0);
        await vi.advanceTimersByTimeAsync(PREWARM_LEASE_MAX_MS);
        expect(lease).toHaveBeenLastCalledWith(null);
        expect(log.mock.calls[0][0]).toContain("[tab-intent] drew pane a1");
        expect(log.mock.calls[0][0]).toContain("(lease cap hit)");
    });

    it("does not draw a pane again that it could not make current in one pass", async () => {
        const { s, lease, pane, settle } = setup();
        s.add(pane("a1", "t1")); // stays stale: its document keeps changing
        s.intent("t1");
        await vi.advanceTimersByTimeAsync(0);
        await settle();
        await vi.advanceTimersByTimeAsync(10);
        expect(lease.mock.calls.map((c) => c[0])).toEqual(["a1", null]);
    });

    it("takes the lease from the idle prewarm, which draws its pane later", async () => {
        const { s, lease, cancelSettle, pane, settle } = setup({ enabled: true });
        const idleOne = pane("idle1", "t9");
        idleOne.tabSeen = () => false;
        idleOne.isDrawable = false; // not an intent candidate
        s.add(idleOne);
        s.add(pane("a1", "t1"));
        await s.tick();
        expect(lease).toHaveBeenLastCalledWith("idle1");
        s.intent("t1");
        await vi.advanceTimersByTimeAsync(0);
        expect(cancelSettle).toHaveBeenCalled();
        expect(lease.mock.calls.map((c) => c[0])).toEqual(["idle1", null, "a1"]);
        await settle();
        // the idle prewarm is free again: its pane gets the next tick
        await s.tick();
        expect(lease).toHaveBeenLastCalledWith("idle1");
    });

    it("reports the panes a tab can draw ahead and how many were, and a show asks once", async () => {
        const { s, pane, settle } = setup();
        s.add(pane("a1", "t1"));
        s.add(pane("a2", "t1"));
        s.intent("t1");
        await vi.advanceTimersByTimeAsync(0);
        await settle();
        s.intent(null);
        expect(s.tabDrawn("t1")).toEqual({ drawn: 1, of: 2 });
        expect(takeDrawnAhead("a1")).toBe(true);
        expect(takeDrawnAhead("a1")).toBe(false);
        expect(s.tabDrawn("t1")).toEqual({ drawn: 0, of: 2 });
    });
});
