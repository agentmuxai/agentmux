// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression tests for the five-hour stuck-"Working…" stall reproduced from a
 * real session (2026-09-09, block f861ac9c: turn ended 23:12:30Z, pane stayed
 * in Streaming until 04:13Z).
 *
 * The mechanism, in three parts — each has a test below:
 *   1. output from a live BACKGROUND shell refreshed the turn idle clock on
 *      every flush, holding it permanently under the recovery bar;
 *   2. a zero-node flush resurrected an already-completed turn;
 *   3. the stuck diagnostic reported the 45s warning bar, not the 180s
 *      recovery bar, so the logs read as "past the threshold, still stuck".
 *
 * Part 1 is the load-bearing one: on its own it kept the pane pinned for as
 * long as the background process lived.
 */

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { initialState, LIVENESS_RECOVERY_MS, STUCK_THRESHOLD_MS } from "./types";

const ready = (at: number) =>
    update(initialState("test-agent"), { type: "StreamSubscribe", at }).state;

/** A live Streaming turn with no tool running — the state the stall sat in. */
const streamingIdle = (at = 1000) => {
    const s = update(ready(at), { type: "TurnStart", at }).state;
    return update(s, { type: "StreamFlushObserved", addedCount: 1, at }).state;
};

describe("background shell output is not turn liveness", () => {
    it("does not refresh the idle clock", () => {
        const s = streamingIdle(1000);
        const before = s.lastEventMs;
        const after = update(s, {
            type: "StreamFlushObserved",
            addedCount: 0,
            at: 50_000,
            turnRelevant: false,
        }).state;
        expect(after.lastEventMs).toBe(before);
    });

    it("lets the watchdog recover a turn that only background output kept alive", () => {
        // The actual stall: a flush every ~50s for hours. Each one reset the
        // clock, so idleSinceMs never reached LIVENESS_RECOVERY_MS and the
        // watchdog declined forever. With the flush marked non-turn-relevant
        // the clock keeps running and recovery fires on schedule.
        let s = streamingIdle(1000);
        for (let t = 50_000; t < LIVENESS_RECOVERY_MS + 1000; t += 50_000) {
            s = update(s, {
                type: "StreamFlushObserved",
                addedCount: 0,
                at: t,
                turnRelevant: false,
            }).state;
        }
        const tick = update(s, {
            type: "StreamWatchdogTick",
            nowMs: 1000 + LIVENESS_RECOVERY_MS + 1,
        });
        expect(tick.state.turnPhase.kind).toBe("Idle");
        expect(tick.events.some((e) => e.type === "working-recovered")).toBe(true);
    });

    it("still emits the flush event so other consumers are unaffected", () => {
        const r = update(streamingIdle(1000), {
            type: "StreamFlushObserved",
            addedCount: 0,
            at: 50_000,
            turnRelevant: false,
        });
        expect(r.events).toEqual([{ type: "stream-flush-observed", addedCount: 0 }]);
    });

    it("treats an unmarked flush as turn-relevant, preserving old behaviour", () => {
        // Only the flush queue knows the difference; every other dispatcher
        // (and every pre-existing test) omits the flag and must be unchanged.
        const s = streamingIdle(1000);
        const after = update(s, { type: "StreamFlushObserved", addedCount: 1, at: 50_000 }).state;
        expect(after.lastEventMs).toBe(50_000);
    });
});

describe("a zero-node flush does not resurrect a completed turn", () => {
    const completed = (at: number) => {
        const s = streamingIdle(at);
        return update(s, { type: "TurnEnd", at: at + 10, outcome: "completed" }).state;
    };

    it("stays Done for a flush that added no document nodes", () => {
        const s = completed(1000);
        expect(s.turnPhase.kind).toBe("Done");
        const after = update(s, { type: "StreamFlushObserved", addedCount: 0, at: 2000 }).state;
        expect(after.turnPhase.kind).toBe("Done");
    });

    it("still promotes for a real next round, which opens with a node", () => {
        // The multi-round tool-continuation case this branch exists for
        // (#2420) must keep working — it always adds an assistant node.
        const s = completed(1000);
        const after = update(s, { type: "StreamFlushObserved", addedCount: 1, at: 2000 }).state;
        expect(after.turnPhase.kind).toBe("Streaming");
    });
});

describe("the stuck diagnostic reports the bar it is measured against", () => {
    it("carries both the warning bar and the recovery bar", () => {
        const s = streamingIdle(1000);
        const r = update(s, {
            type: "StreamWatchdogTick",
            nowMs: 1000 + STUCK_THRESHOLD_MS + 1,
        });
        const stuck = r.events.find((e) => e.type === "stream-stuck");
        expect(stuck).toBeDefined();
        // Reporting only thresholdMs made the logs read as "idle exceeded the
        // threshold and it still did not recover" — the misreading that cost a
        // day of diagnosis. The two bars are 45s and 180s and differ by 4x.
        expect(stuck).toMatchObject({
            thresholdMs: STUCK_THRESHOLD_MS,
            recoverThresholdMs: LIVENESS_RECOVERY_MS,
        });
        expect(r.state.turnPhase.kind).toBe("Streaming");
    });
});
