// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for the tab-content reveal gate (issue #774).
// Specifically guards the holdRevealGate / scheduleRevealLift pairing:
// during async work without longtasks (RPCs, layout-model polling),
// the SETTLE window must NOT elapse and prematurely reveal the tab.

import { describe, test, expect, beforeEach, afterEach, vi } from "vitest";
import { createRoot } from "solid-js";
import { holdRevealGate, scheduleRevealLift, tabSwitching } from "./tab-reveal";

function read<T>(signal: () => T): T {
    let val!: T;
    createRoot((dispose) => {
        val = signal();
        dispose();
    });
    return val;
}

// All PerformanceObserver instances installed by scheduleRevealLift land
// here so tests can drive the rAF tick path without a real browser.
// `requestAnimationFrame` is stubbed by vi.useFakeTimers below.

describe("tab-reveal gate", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        // Force the no-PerformanceObserver fallback path so the gate
        // lift behaviour is entirely timer-driven and deterministic.
        // The real production path uses PerformanceObserver longtask
        // entries; the SETTLE-elapsed-during-await bug this test
        // guards reproduces identically in the fallback path because
        // both rely on SETTLE_MS elapsing without "busy" signal.
        // Strip PerformanceObserver for the fallback path (it is not
        // strictly typed on globalThis under our tsconfig, so a plain
        // assignment is fine — no ts-expect-error needed).
        (globalThis as { PerformanceObserver?: unknown }).PerformanceObserver = undefined;
    });

    afterEach(() => {
        // Reset so subsequent tests start with the gate down.
        // scheduleRevealLift + fast-forward triggers the fallback
        // timer (MAX_GATE_MS=800) to clear the signal; advance past
        // it to drain.
        scheduleRevealLift();
        vi.advanceTimersByTime(1000);
        vi.useRealTimers();
    });

    test("holdRevealGate raises the gate", () => {
        expect(read(tabSwitching)).toBe(false);
        holdRevealGate();
        expect(read(tabSwitching)).toBe(true);
    });

    test("holdRevealGate keeps the gate up across long awaits", () => {
        // Simulates the createTab / setActiveTab failure mode: an RPC
        // that runs longer than SETTLE_MS (80ms) with no longtasks
        // firing. Under the old code (scheduleRevealLift before await),
        // the fallback timer would lift the gate after 80ms even
        // though the destination tab had not yet mounted.
        holdRevealGate();
        vi.advanceTimersByTime(500); // way past SETTLE_MS, still under MAX_GATE_MS
        expect(read(tabSwitching)).toBe(true);
    });

    test("holdRevealGate safety-lifts after MAX_GATE_MS if no schedule follows", () => {
        // Codex P2: if the awaited RPC never settles (callBackendService
        // has no timeout), the paired scheduleRevealLift in `finally`
        // never runs and the gate would stay up forever, leaving the
        // window blank indefinitely. The safety net inside
        // holdRevealGate prevents this — gate auto-lifts at the hard
        // cap even with no paired schedule.
        holdRevealGate();
        vi.advanceTimersByTime(900); // past MAX_GATE_MS=800
        expect(read(tabSwitching)).toBe(false);
    });

    test("scheduleRevealLift after holdRevealGate eventually lifts via fallback", () => {
        holdRevealGate();
        vi.advanceTimersByTime(500);
        expect(read(tabSwitching)).toBe(true);
        // Pair the hold with a schedule once the simulated async work
        // completes. The fallback timer arms now (MAX_GATE_MS=800ms)
        // and fires, dropping the gate.
        scheduleRevealLift();
        // Still well within the hard cap — gate stays up.
        vi.advanceTimersByTime(400);
        expect(read(tabSwitching)).toBe(true);
        // Past MAX_GATE_MS — fallback fires, gate drops.
        vi.advanceTimersByTime(500);
        expect(read(tabSwitching)).toBe(false);
    });

    test("holdRevealGate cancels a pending fallback timer from a prior schedule", () => {
        // The same stale-fallback-timer class of bug that PR commit
        // 986c92ba fixed for re-entry of scheduleRevealLift — verify
        // holdRevealGate also cancels it. Otherwise a hold issued
        // shortly after a schedule could see the stale timer fire and
        // drop the gate mid-await.
        scheduleRevealLift();
        // Get well into the original fallback window, then re-enter
        // via holdRevealGate.
        vi.advanceTimersByTime(400);
        holdRevealGate();
        // Past where the prior schedule's MAX_GATE_MS would have fired
        // (800ms from the schedule call, i.e. 400ms after the hold).
        // Stay under the hold's OWN safety net so we're verifying the
        // prior timer's cancellation, not gate-still-up by other means.
        vi.advanceTimersByTime(500);
        expect(read(tabSwitching)).toBe(true);
    });

    test("rapid hold→schedule→hold sequence keeps the gate up", () => {
        // setActiveTab spam case: each call holds, awaits, then
        // schedules. A subsequent call must re-hold before the prior
        // call's schedule fallback timer fires.
        holdRevealGate();
        vi.advanceTimersByTime(30);
        scheduleRevealLift();
        // Well within MAX_GATE_MS, so the gate is still up.
        vi.advanceTimersByTime(400);
        expect(read(tabSwitching)).toBe(true);
        holdRevealGate();
        // Past where the prior schedule's fallback would have fired,
        // but under the hold's own safety net.
        vi.advanceTimersByTime(500);
        expect(read(tabSwitching)).toBe(true);
    });
});
