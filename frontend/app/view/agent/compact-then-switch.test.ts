// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { compactThenSwitch } from "./compact-then-switch";

/** `boundaries` is what boundaryAt answers on each call (the last one repeats); `busy` is what turnBusy answers on each poll. */
function harness(boundaries: Array<number | null>, busy: boolean[] = [true]) {
    const calls: string[] = [];
    let reads = 0;
    let busyReads = 0;
    let clock = 0;
    return {
        calls,
        deps: {
            compact: async () => {
                calls.push("compact");
            },
            boundaryAt: () => boundaries[Math.min(reads++, boundaries.length - 1)],
            turnBusy: () => busy[Math.min(busyReads++, busy.length - 1)],
            switchTo: async () => {
                calls.push("switch");
            },
            wait: async (ms: number) => {
                clock += ms;
            },
            now: () => clock,
            pollMs: 100,
            timeoutMs: 1000,
        },
    };
}

describe("compactThenSwitch", () => {
    it("switches once the compaction boundary has moved forward", async () => {
        // boundaryAt: once before the compact, then while waiting
        const h = harness([null, null, 5]);
        await compactThenSwitch(h.deps);
        expect(h.calls).toEqual(["compact", "switch"]);
    });

    it("does not switch when the conversation is not compacted in time", async () => {
        const h = harness([7], [true]);
        await expect(compactThenSwitch(h.deps)).rejects.toThrow(/not compacted in time/);
        expect(h.calls).toEqual(["compact"]);
    });

    it("stops waiting when the compaction's turn ends with no boundary (it failed, or was interrupted)", async () => {
        // The turn is in flight for two polls, then the pane is idle and the boundary never moved.
        const h = harness([7], [true, true, false]);
        await expect(compactThenSwitch(h.deps)).rejects.toThrow(/did not finish/);
        expect(h.calls).toEqual(["compact"]);
    });

    it("does not switch on a later compaction once the requested one has ended", async () => {
        // Idle for three polls after the turn: the wait is over. A boundary that moves afterwards is nobody's request.
        const h = harness([7, 7, 7, 7, 7, 7, 9], [true, false]);
        await expect(compactThenSwitch(h.deps)).rejects.toThrow(/did not finish/);
        expect(h.calls).toEqual(["compact"]);
    });

    it("takes the boundary when it lands just after the turn went idle", async () => {
        const h = harness([7, 7, 7, 9], [true, false, false]);
        await compactThenSwitch(h.deps);
        expect(h.calls).toEqual(["compact", "switch"]);
    });

    it("does not give up on a pane that has not started the turn yet (the compaction is queued)", async () => {
        const h = harness([7, 7, 7, 7, 7, 7, 7, 7, 8], [false]);
        await compactThenSwitch(h.deps);
        expect(h.calls).toEqual(["compact", "switch"]);
    });

    it("does not take a boundary that is not later than the one the pane had", async () => {
        const h = harness([7, 3], [true]);
        await expect(compactThenSwitch(h.deps)).rejects.toThrow(/not compacted in time/);
        expect(h.calls).toEqual(["compact"]);
    });

    it("lets a failed switch reach the caller", async () => {
        const h = harness([1, 2]);
        h.deps.switchTo = async () => {
            throw new Error("KEY_MISSING");
        };
        await expect(compactThenSwitch(h.deps)).rejects.toThrow("KEY_MISSING");
    });
});
