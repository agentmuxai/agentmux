// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * createDebouncedRefresh — see docs/specs/SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md.
 *
 * All timer advances below deliberately land a comfortable margin before/
 * after each deadline (never exactly on one) to avoid relying on fake-timer
 * inclusive/exclusive boundary semantics.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createDebouncedRefresh } from "./debounced-refresh";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("createDebouncedRefresh", () => {
    it("collapses a rapid burst of triggers into exactly one call, after the wait window", () => {
        const fn = vi.fn();
        const trigger = createDebouncedRefresh(fn, 100, 1000);

        for (let i = 0; i < 50; i++) trigger();
        expect(fn).not.toHaveBeenCalled();

        vi.advanceTimersByTime(90);
        expect(fn).not.toHaveBeenCalled();
        vi.advanceTimersByTime(20); // total 110 — past the 100ms trailing deadline
        expect(fn).toHaveBeenCalledTimes(1);
    });

    it("a trigger within the wait window resets the timer instead of adding a second call", () => {
        const fn = vi.fn();
        const trigger = createDebouncedRefresh(fn, 100, 1000);

        trigger(); // trailing deadline: 100
        vi.advanceTimersByTime(80); // before the original deadline
        trigger(); // resets — new trailing deadline: 80 + 100 = 180
        vi.advanceTimersByTime(80); // total 160 — before the new deadline
        expect(fn).not.toHaveBeenCalled();
        vi.advanceTimersByTime(30); // total 190 — past 180
        expect(fn).toHaveBeenCalledTimes(1);
    });

    it("the max-wait ceiling forces a call even under continuous triggering faster than the wait window", () => {
        const fn = vi.fn();
        const trigger = createDebouncedRefresh(fn, 100, 1000);

        trigger(); // max deadline fixed at 1000
        // Re-trigger every 60ms (< the 100ms wait) so the trailing timer
        // never gets a chance to fire on its own, for ~960ms total —
        // comfortably short of the 1000ms ceiling.
        for (let i = 0; i < 16; i++) {
            vi.advanceTimersByTime(60);
            trigger();
        }
        expect(fn).not.toHaveBeenCalled(); // t=960, ceiling is at 1000

        vi.advanceTimersByTime(60); // total 1020 — past the ceiling
        expect(fn).toHaveBeenCalledTimes(1);
    });

    it("firing via the ceiling does not ALSO fire the trailing timer shortly after", () => {
        const fn = vi.fn();
        const trigger = createDebouncedRefresh(fn, 100, 1000);

        trigger();
        for (let i = 0; i < 16; i++) {
            vi.advanceTimersByTime(60);
            trigger();
        }
        vi.advanceTimersByTime(60); // crosses the ceiling — fires once
        expect(fn).toHaveBeenCalledTimes(1);

        // No further triggers — nothing else should fire later, even
        // though the last trigger's own trailing deadline would otherwise
        // still be pending in this range had `fire()` not cleared it.
        vi.advanceTimersByTime(2000);
        expect(fn).toHaveBeenCalledTimes(1);
    });

    it("independent instances never interfere with each other", () => {
        const fnA = vi.fn();
        const fnB = vi.fn();
        const triggerA = createDebouncedRefresh(fnA, 100, 1000);
        const triggerB = createDebouncedRefresh(fnB, 100, 1000);

        triggerA(); // A's trailing deadline: 100
        vi.advanceTimersByTime(50);
        triggerB(); // B's trailing deadline: 50 + 100 = 150
        vi.advanceTimersByTime(70); // total 120 — past A's deadline, before B's
        expect(fnA).toHaveBeenCalledTimes(1);
        expect(fnB).not.toHaveBeenCalled();

        vi.advanceTimersByTime(50); // total 170 — past B's deadline
        expect(fnB).toHaveBeenCalledTimes(1);
    });

    it("a new burst after a prior one settled starts a fresh cycle", () => {
        const fn = vi.fn();
        const trigger = createDebouncedRefresh(fn, 100, 1000);

        trigger();
        vi.advanceTimersByTime(110);
        expect(fn).toHaveBeenCalledTimes(1);

        trigger();
        vi.advanceTimersByTime(110);
        expect(fn).toHaveBeenCalledTimes(2);
    });
});
