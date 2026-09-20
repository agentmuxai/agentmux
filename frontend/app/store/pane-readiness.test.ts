// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneReadiness (SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md phase 1).
 *
 * The headline property this exists to guarantee: ONE moment at which a pane is
 * allowed to appear, instead of several components each deciding independently.
 * So the assertions are mostly about ordering and about the failure modes a single
 * authority introduces — a stuck gate must not hide the pane forever, and a late
 * gate must not re-hide a pane already on screen.
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPaneReadiness } from "./pane-readiness";

/** Runs `fn` inside a root so onCleanup/signals behave as in a component. */
function inRoot<T>(fn: () => T): { value: T; dispose: () => void } {
    let value!: T;
    const dispose = createRoot((d) => {
        value = fn();
        return d;
    });
    return { value, dispose };
}

describe("createPaneReadiness", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("reveals immediately when no gate is ever registered", async () => {
        const { value: r, dispose } = inRoot(() => createPaneReadiness());
        expect(r.phase()).toBe("assembling");
        await Promise.resolve(); // the queueMicrotask escape hatch
        expect(r.phase()).toBe("revealing");
        dispose();
    });

    it("stays assembling until every gate completes, then reveals once", async () => {
        const { value: r, dispose } = inRoot(() => createPaneReadiness());
        const history = r.gate("history");
        const auth = r.gate("auth");
        await Promise.resolve();

        expect(r.phase()).toBe("assembling");
        expect(r.pendingGates().sort()).toEqual(["auth", "history"]);

        history();
        expect(r.phase()).toBe("assembling"); // auth still outstanding
        expect(r.pendingGates()).toEqual(["auth"]);

        auth();
        expect(r.phase()).toBe("revealing");
        expect(r.pendingGates()).toEqual([]);
        dispose();
    });

    it("moves to live only when the cover reports its fade finished", () => {
        const { value: r, dispose } = inRoot(() => createPaneReadiness());
        const done = r.gate("only");
        done();
        expect(r.phase()).toBe("revealing");
        expect(r.isLoading()).toBe(true); // still covered during the fade

        r.revealComplete();
        expect(r.phase()).toBe("live");
        expect(r.isLoading()).toBe(false);
        dispose();
    });

    it("is idempotent — completing the same gate twice does not release another", () => {
        const { value: r, dispose } = inRoot(() => createPaneReadiness());
        const a = r.gate("a");
        r.gate("b");
        a();
        a();
        a();
        expect(r.phase()).toBe("assembling");
        expect(r.pendingGates()).toEqual(["b"]);
        dispose();
    });

    /**
     * THE behaviour-preservation test, and the one that matters most.
     *
     * The code this replaces waited indefinitely for its conditions. A slow
     * persisted-session pane (large transcript replay, auth settling, subagent
     * backfill) is the legitimate slow case AND the one the cover exists for, so
     * force-revealing it after N seconds would expose exactly the half-assembled
     * pane this consolidation eliminates. Exercises the DEFAULT — no override —
     * because that is what every real call site uses. (reagent P1 on #3462.)
     */
    it("by default never force-reveals, however long a gate takes", () => {
        const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        const { value: r, dispose } = inRoot(() => createPaneReadiness({ label: "block:slow" }));
        const history = r.gate("history");
        r.gate("auth");

        vi.advanceTimersByTime(10 * 60 * 1000); // ten minutes
        expect(r.phase()).toBe("assembling"); // still covered, as before
        expect(r.pendingGates().sort()).toEqual(["auth", "history"]);

        history();
        expect(r.phase()).toBe("assembling"); // auth still outstanding
        errSpy.mockRestore();
        dispose();
    });

    it("warns loudly about stuck gates without revealing them away", () => {
        const onTimeout = vi.fn();
        const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        const { value: r, dispose } = inRoot(() =>
            createPaneReadiness({ warnAfterMs: 5000, label: "block:abc", onTimeout })
        );
        r.gate("history");
        r.gate("never-completes");

        vi.advanceTimersByTime(5001);

        expect(onTimeout).toHaveBeenCalledWith(expect.arrayContaining(["history", "never-completes"]));
        expect(errSpy).toHaveBeenCalledWith(expect.stringContaining("never-completes"));
        expect(errSpy).toHaveBeenCalledWith(expect.stringContaining("cover stays up"));
        expect(r.phase()).toBe("assembling"); // warned, NOT revealed
        errSpy.mockRestore();
        dispose();
    });

    it("force-reveals only when a caller explicitly opts in via revealTimeoutMs", () => {
        const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        const { value: r, dispose } = inRoot(() =>
            createPaneReadiness({ revealTimeoutMs: 5000, label: "block:bounded" })
        );
        r.gate("never-completes");

        vi.advanceTimersByTime(5001);
        expect(r.phase()).toBe("revealing");
        expect(errSpy).toHaveBeenCalledWith(expect.stringContaining("Forcing reveal"));
        errSpy.mockRestore();
        dispose();
    });

    /**
     * The warn and reveal deadlines are INDEPENDENT. An earlier revision armed a
     * single timer at `Math.min(warnAfterMs, revealTimeoutMs)` and force-revealed
     * whenever it fired, so a caller asking for a 20s hard bound got revealed at
     * the 8s default warn instead — a silent violation of the bound it requested.
     * Only `revealTimeoutMs < warnAfterMs` was covered, which is why it survived.
     * (reagent P1 on #3462, round 2.)
     */
    it("honours a revealTimeoutMs LONGER than warnAfterMs", () => {
        const onTimeout = vi.fn();
        const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        const { value: r, dispose } = inRoot(() =>
            createPaneReadiness({ warnAfterMs: 8000, revealTimeoutMs: 20000, label: "block:slow", onTimeout })
        );
        r.gate("history");

        vi.advanceTimersByTime(8001); // warn deadline only
        expect(onTimeout).toHaveBeenCalledTimes(1);
        expect(r.phase()).toBe("assembling"); // warned, NOT revealed at 8s

        vi.advanceTimersByTime(11000); // still short of 20s
        expect(r.phase()).toBe("assembling");

        vi.advanceTimersByTime(1001); // now past the caller's bound
        expect(r.phase()).toBe("revealing");
        expect(errSpy).toHaveBeenCalledWith(expect.stringContaining("Forcing reveal"));
        errSpy.mockRestore();
        dispose();
    });

    it("warns once, not once per gate registered after the deadline", () => {
        const onTimeout = vi.fn();
        const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        const { value: r, dispose } = inRoot(() => createPaneReadiness({ warnAfterMs: 5000, onTimeout }));
        r.gate("history");
        vi.advanceTimersByTime(5001);
        expect(onTimeout).toHaveBeenCalledTimes(1);

        r.gate("late"); // must not re-arm the deadline
        vi.advanceTimersByTime(60000);
        expect(onTimeout).toHaveBeenCalledTimes(1);
        errSpy.mockRestore();
        dispose();
    });

    it("does not warn once every gate completed in time", () => {
        const onTimeout = vi.fn();
        const { value: r, dispose } = inRoot(() => createPaneReadiness({ warnAfterMs: 5000, onTimeout }));
        const done = r.gate("history");
        done();
        vi.advanceTimersByTime(10000);
        expect(onTimeout).not.toHaveBeenCalled();
        expect(r.phase()).toBe("revealing");
        dispose();
    });

    /**
     * A dependency that shows up after the pane is already visible must not yank
     * the cover back over content the user is reading.
     */
    it("ignores a gate registered after the pane has revealed", () => {
        const { value: r, dispose } = inRoot(() => createPaneReadiness());
        r.gate("first")();
        expect(r.phase()).toBe("revealing");

        const late = r.gate("late-arrival");
        expect(r.phase()).toBe("revealing");
        expect(r.pendingGates()).toEqual([]);
        late(); // must not throw or regress the phase
        expect(r.phase()).toBe("revealing");
        dispose();
    });

    it("clears its timeout on dispose so a closed pane cannot fire it", () => {
        const onTimeout = vi.fn();
        const { value: r, dispose } = inRoot(() => createPaneReadiness({ warnAfterMs: 1000, onTimeout }));
        r.gate("history");
        dispose();
        vi.advanceTimersByTime(5000);
        expect(onTimeout).not.toHaveBeenCalled();
    });
});
