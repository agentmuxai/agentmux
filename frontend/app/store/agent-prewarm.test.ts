// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPrewarmScheduler, PREWARM_LEASE_MAX_MS, type Pressure, type PrewarmCandidate } from "./agent-prewarm";

function setup(over: { enabled?: boolean; idle?: boolean; pressure?: Pressure } = {}) {
    const state = { enabled: over.enabled ?? true, idle: over.idle ?? true, pressure: over.pressure ?? ("normal" as Pressure), heap: 100 * 1048576 };
    let settleDone: (() => void) | null = null;
    const lease = vi.fn();
    const log = vi.fn();
    const cancelSettle = vi.fn();
    const s = createPrewarmScheduler({
        enabled: () => state.enabled,
        idle: () => state.idle,
        pressure: async () => state.pressure,
        heapBytes: () => state.heap,
        lease,
        settle: (done) => {
            settleDone = done;
            return cancelSettle;
        },
        log,
        now: () => 0,
    });
    const pane = (blockId: string, tabId: string, seen = false): PrewarmCandidate & { seen: boolean; nodes: number } => {
        const c = { blockId, tabId, seen, nodes: 10, tabSeen: () => c.seen, nodeCount: () => c.nodes };
        return c;
    };
    return { s, state, lease, log, cancelSettle, finish: () => settleDone?.(), pane };
}

describe("agent pane prewarm", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("leases one hidden pane, releases it when it has settled, and logs what it cost", async () => {
        const { s, state, lease, log, finish, pane } = setup();
        const a = pane("aaaaaaaa-1", "tab-b");
        s.add(a);
        await s.tick();
        expect(lease).toHaveBeenLastCalledWith("aaaaaaaa-1");
        a.nodes = 510;
        state.heap += 8 * 1048576;
        finish();
        expect(lease).toHaveBeenLastCalledWith(null);
        expect(log).toHaveBeenCalledTimes(1);
        expect(log.mock.calls[0][0]).toContain("500 nodes");
        expect(log.mock.calls[0][0]).toContain("grew 8.0 MB");
        await s.tick();
        expect(lease).toHaveBeenCalledTimes(2); // not leased again
        expect(s.warmedCount()).toBe(1);
    });

    it("does not start another pane while one holds the lease", async () => {
        const { s, lease, pane } = setup();
        s.add(pane("a", "t1"));
        s.add(pane("b", "t1"));
        await s.tick();
        await s.tick();
        expect(lease).toHaveBeenCalledTimes(1);
    });

    it("finishes a tab before starting the next one", async () => {
        const { s, lease, finish, pane } = setup();
        s.add(pane("a1", "t1"));
        s.add(pane("b1", "t2"));
        s.add(pane("a2", "t1"));
        await s.tick();
        finish();
        await s.tick();
        finish();
        await s.tick();
        const leased = lease.mock.calls.map((c) => c[0]).filter(Boolean);
        expect(leased).toEqual(["a1", "a2", "b1"]);
    });

    it("skips panes whose tab is shown or was shown before", async () => {
        const { s, lease, pane } = setup();
        s.add(pane("seen", "t1", true));
        await s.tick();
        expect(lease).not.toHaveBeenCalled();
    });

    it("waits while the setting is off, the window is in use, or memory is under pressure", async () => {
        const { s, state, lease, pane } = setup({ enabled: false });
        s.add(pane("a", "t1"));
        await s.tick();
        state.enabled = true;
        state.idle = false;
        await s.tick();
        state.idle = true;
        state.pressure = "warn";
        await s.tick();
        state.pressure = "critical";
        await s.tick();
        expect(lease).not.toHaveBeenCalled();
        state.pressure = "normal";
        await s.tick();
        expect(lease).toHaveBeenCalledWith("a");
    });

    it("does not lease when the person starts using the window while the pressure is being read", async () => {
        const { s, state, lease, pane } = setup();
        s.add(pane("a", "t1"));
        const p = s.tick();
        state.idle = false;
        await p;
        expect(lease).not.toHaveBeenCalled();
    });

    it("ends the lease at the cap when the pane never settles, and says so", async () => {
        const { s, lease, log, cancelSettle, pane } = setup();
        s.add(pane("a", "t1"));
        await s.tick();
        vi.advanceTimersByTime(PREWARM_LEASE_MAX_MS);
        expect(cancelSettle).toHaveBeenCalled();
        expect(lease).toHaveBeenLastCalledWith(null);
        expect(log.mock.calls[0][0]).toContain("lease cap hit");
    });

    it("gives the lease back when the pane closes mid-lease", async () => {
        const { s, lease, pane } = setup();
        const remove = s.add(pane("a", "t1"));
        await s.tick();
        remove();
        expect(lease).toHaveBeenLastCalledWith(null);
        s.add(pane("b", "t1"));
        await s.tick();
        expect(lease).toHaveBeenLastCalledWith("b");
    });
});
