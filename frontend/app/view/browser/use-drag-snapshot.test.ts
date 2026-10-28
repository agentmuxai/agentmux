// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { createSlotGate, snapshotFitsPane } from "./use-drag-snapshot";

describe("snapshotFitsPane", () => {
    const pane = { x: 0, y: 0, width: 815, height: 463 };

    it("accepts this pane's own size, allowing for rounding", () => {
        expect(snapshotFitsPane({ width: 815, height: 463 }, pane)).toBe(true);
        expect(snapshotFitsPane({ width: 816, height: 464 }, pane)).toBe(true);
    });

    // Measured live: with several panes on one URL, the host returned
    // another pane's 815x604 page for this 815x463 pane.
    it("rejects another pane's page", () => {
        expect(snapshotFitsPane({ width: 815, height: 604 }, pane)).toBe(false);
        expect(snapshotFitsPane({ width: 986, height: 463 }, pane)).toBe(false);
    });
});

describe("createSlotGate", () => {
    it("runs at most `max` jobs at once and starts the rest in order as slots free up", async () => {
        const gate = createSlotGate(2);
        let running = 0;
        let peak = 0;
        const started: number[] = [];
        const releases: Array<() => void> = [];
        const job = (n: number) =>
            gate(async () => {
                started.push(n);
                running++;
                peak = Math.max(peak, running);
                await new Promise<void>((done) => releases.push(done));
                running--;
                return n;
            });
        const all = [1, 2, 3, 4, 5].map(job);
        await Promise.resolve();
        expect(started).toEqual([1, 2]);
        for (let i = 0; i < 5; i++) {
            releases[i]?.();
            await new Promise((r) => setTimeout(r, 0));
        }
        expect(await Promise.all(all)).toEqual([1, 2, 3, 4, 5]);
        expect(started).toEqual([1, 2, 3, 4, 5]);
        expect(peak).toBe(2);
    });

    it("frees the slot when a job throws", async () => {
        const gate = createSlotGate(1);
        await expect(gate(async () => Promise.reject(new Error("no")))).rejects.toThrow("no");
        expect(await gate(async () => "after")).toBe("after");
    });
});
