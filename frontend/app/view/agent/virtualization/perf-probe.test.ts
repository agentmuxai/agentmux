// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the agent-pane perf probe. Note: these tests run under
 * Vitest's `import.meta.env.DEV === true`, so probing IS active here.
 * Production no-op behavior is verified by inspecting the
 * `isProbingEnabled()` short-circuit at the top of each record fn —
 * not by a test (Vitest doesn't simulate the prod env without a
 * separate config, and the bundler tree-shakes the dead branch).
 */

import { beforeEach, describe, expect, it } from "vitest";
import { sleep } from "@/util/util";
import {
    agentPerfStore,
    ESTIMATOR_MISS_THRESHOLD,
    markDispatch,
    markRowMount,
} from "./perf-probe";

describe("agentPerfStore", () => {
    beforeEach(() => agentPerfStore.reset());

    describe("recordRowMount", () => {
        it("aggregates per-kind durations into a snapshot", () => {
            agentPerfStore.recordRowMount("agent_message", 5);
            agentPerfStore.recordRowMount("agent_message", 10);
            agentPerfStore.recordRowMount("tool", 2);

            const snap = agentPerfStore.snapshot();
            expect(snap.rowMountByKind.has("agent_message")).toBe(true);
            expect(snap.rowMountByKind.has("tool")).toBe(true);
            expect(snap.rowMountByKind.get("agent_message")!.count).toBe(2);
            expect(snap.rowMountByKind.get("tool")!.count).toBe(1);
        });
    });

    describe("recordEstimatorMeasurement", () => {
        it("does not flag measurements within threshold", () => {
            // estimated 100, actual 110 → 10% error, under 30% threshold
            agentPerfStore.recordEstimatorMeasurement("markdown", 100, 110);
            const snap = agentPerfStore.snapshot();
            expect(snap.recentEstimatorMisses).toHaveLength(0);
            expect(snap.estimatorMissRateByKind.get("markdown")).toBe(0);
        });

        it("flags measurements outside threshold and records miss-rate", () => {
            // estimated 100, actual 200 → 100% error
            agentPerfStore.recordEstimatorMeasurement("tool", 100, 200);
            const snap = agentPerfStore.snapshot();
            expect(snap.recentEstimatorMisses).toHaveLength(1);
            expect(snap.recentEstimatorMisses[0]).toMatchObject({
                kind: "tool",
                estimated: 100,
                actual: 200,
                errorPct: 1,
            });
            expect(snap.estimatorMissRateByKind.get("tool")).toBe(1);
        });

        it("computes miss-rate correctly across measurements", () => {
            // 1 miss, 3 hits → miss rate 0.25
            agentPerfStore.recordEstimatorMeasurement("section", 50, 100); // 100% error → miss
            agentPerfStore.recordEstimatorMeasurement("section", 50, 55);  // 10% → hit
            agentPerfStore.recordEstimatorMeasurement("section", 50, 60);  // 20% → hit
            agentPerfStore.recordEstimatorMeasurement("section", 50, 50);  // 0% → hit
            const snap = agentPerfStore.snapshot();
            expect(snap.estimatorMissRateByKind.get("section")).toBeCloseTo(0.25, 2);
        });

        it("rejects measurements with estimated=0 (avoid divide-by-zero)", () => {
            agentPerfStore.recordEstimatorMeasurement("shell", 0, 100);
            // errorPct should be 0 (defensive), not Infinity
            const snap = agentPerfStore.snapshot();
            // The measurement still counted toward total, just not flagged.
            expect(snap.estimatorMissRateByKind.get("shell")).toBe(0);
        });

        it("uses the documented threshold constant", () => {
            // Just over threshold → flagged
            const justOver = ESTIMATOR_MISS_THRESHOLD + 0.01;
            agentPerfStore.recordEstimatorMeasurement("user_message", 100, 100 + 100 * justOver);
            expect(agentPerfStore.snapshot().recentEstimatorMisses).toHaveLength(1);

            agentPerfStore.reset();

            // Just under threshold → not flagged
            const justUnder = ESTIMATOR_MISS_THRESHOLD - 0.01;
            agentPerfStore.recordEstimatorMeasurement("user_message", 100, 100 + 100 * justUnder);
            expect(agentPerfStore.snapshot().recentEstimatorMisses).toHaveLength(0);
        });
    });

    describe("recordLayoutShift", () => {
        it("records shifts in chronological-recent order", () => {
            agentPerfStore.recordLayoutShift(0.01);
            agentPerfStore.recordLayoutShift(0.05);
            agentPerfStore.recordLayoutShift(0.02);
            const shifts = agentPerfStore.snapshot().recentLayoutShifts;
            expect(shifts).toHaveLength(3);
            // Most recent first.
            expect(shifts[0].value).toBe(0.02);
            expect(shifts[2].value).toBe(0.01);
        });
    });

    describe("ring buffer bounds", () => {
        it("caps recentEstimatorMisses at the ring size", () => {
            for (let i = 0; i < 100; i++) {
                agentPerfStore.recordEstimatorMeasurement("markdown", 100, 1000);
            }
            // Default SAMPLE_RING_SIZE is 64; verify cap.
            expect(agentPerfStore.snapshot().recentEstimatorMisses.length).toBeLessThanOrEqual(64);
        });

        it("caps recentLayoutShifts at the ring size", () => {
            for (let i = 0; i < 100; i++) {
                agentPerfStore.recordLayoutShift(0.01);
            }
            expect(agentPerfStore.snapshot().recentLayoutShifts.length).toBeLessThanOrEqual(64);
        });
    });

    describe("recordDispatchTiming", () => {
        // task #40 — the store-dispatch timing probe, added to close the
        // blind spot from the original investigation: row-mount timing
        // never measured the reducer/store cost underneath the DOM.
        it("aggregates per-kind dispatch durations into a snapshot", () => {
            agentPerfStore.recordDispatchTiming("layout", 1);
            agentPerfStore.recordDispatchTiming("layout", 3);
            agentPerfStore.recordDispatchTiming("document", 2);

            const snap = agentPerfStore.snapshot();
            expect(snap.dispatchByKind.get("layout")!.count).toBe(2);
            expect(snap.dispatchByKind.get("document")!.count).toBe(1);
        });

        it("keeps layout and document timings independent", () => {
            for (let i = 0; i < 5; i++) agentPerfStore.recordDispatchTiming("layout", 1);
            for (let i = 0; i < 3; i++) agentPerfStore.recordDispatchTiming("document", 1);
            const snap = agentPerfStore.snapshot();
            expect(snap.dispatchByKind.get("layout")!.count).toBe(5);
            expect(snap.dispatchByKind.get("document")!.count).toBe(3);
        });
    });

    describe("reset", () => {
        it("clears all recorded data", () => {
            agentPerfStore.recordRowMount("markdown", 5);
            agentPerfStore.recordEstimatorMeasurement("tool", 100, 200);
            agentPerfStore.recordLayoutShift(0.05);
            agentPerfStore.recordDispatchTiming("layout", 4);

            agentPerfStore.reset();

            const snap = agentPerfStore.snapshot();
            expect(snap.rowMountByKind.size).toBe(0);
            expect(snap.recentEstimatorMisses).toHaveLength(0);
            expect(snap.recentLayoutShifts).toHaveLength(0);
            expect(snap.estimatorMissRateByKind.size).toBe(0);
            expect(snap.dispatchByKind.size).toBe(0);
        });
    });
});

describe("markRowMount", () => {
    beforeEach(() => agentPerfStore.reset());

    it("returns a closer that records duration on call", async () => {
        const close = markRowMount("markdown");
        await sleep(5);
        close();
        const snap = agentPerfStore.snapshot();
        const q = snap.rowMountByKind.get("markdown");
        expect(q).toBeDefined();
        expect(q!.count).toBe(1);
        // Should be at least 5ms (with timer slop, allow 2ms floor).
        expect(q!.max).toBeGreaterThanOrEqual(2);
    });

    it("multiple calls accumulate independently", () => {
        markRowMount("agent_message")();
        markRowMount("agent_message")();
        markRowMount("tool")();
        const snap = agentPerfStore.snapshot();
        expect(snap.rowMountByKind.get("agent_message")!.count).toBe(2);
        expect(snap.rowMountByKind.get("tool")!.count).toBe(1);
    });
});

describe("markDispatch", () => {
    beforeEach(() => agentPerfStore.reset());

    it("returns a closer that records duration on call", async () => {
        const close = markDispatch("layout");
        await sleep(5);
        close();
        const snap = agentPerfStore.snapshot();
        const q = snap.dispatchByKind.get("layout");
        expect(q).toBeDefined();
        expect(q!.count).toBe(1);
        // Should be at least 5ms (with timer slop, allow 2ms floor).
        expect(q!.max).toBeGreaterThanOrEqual(2);
    });

    it("multiple calls accumulate independently per kind", () => {
        markDispatch("layout")();
        markDispatch("layout")();
        markDispatch("document")();
        const snap = agentPerfStore.snapshot();
        expect(snap.dispatchByKind.get("layout")!.count).toBe(2);
        expect(snap.dispatchByKind.get("document")!.count).toBe(1);
    });
});
