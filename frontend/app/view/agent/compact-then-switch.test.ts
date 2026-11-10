// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { compactThenSwitch } from "./compact-then-switch";

function harness(boundaries: Array<number | null>) {
    const calls: string[] = [];
    let polls = 0;
    let clock = 0;
    return {
        calls,
        deps: {
            compact: async () => {
                calls.push("compact");
            },
            boundaryAt: () => boundaries[Math.min(polls++, boundaries.length - 1)],
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
    it("switches once the compaction boundary has moved", async () => {
        // boundaryAt: once before the compact, then twice while waiting
        const h = harness([null, null, 5]);
        await compactThenSwitch(h.deps);
        expect(h.calls).toEqual(["compact", "switch"]);
    });

    it("does not switch when the conversation is not compacted in time", async () => {
        const h = harness([7]);
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
