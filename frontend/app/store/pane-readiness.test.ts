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
     * The failure mode a single authority introduces: one gate that never reports
     * would hide the pane forever. It must degrade to today's behaviour (pane
     * visible) rather than an indefinite cover, and say which gate was stuck.
     */
    it("reveals anyway after the timeout, naming the stuck gates", () => {
        const onTimeout = vi.fn();
        const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        const { value: r, dispose } = inRoot(() =>
            createPaneReadiness({ revealTimeoutMs: 5000, label: "block:abc", onTimeout })
        );
        r.gate("history");
        r.gate("never-completes");
        r.gate("history-2")();

        expect(r.phase()).toBe("assembling");
        vi.advanceTimersByTime(5001);

        expect(r.phase()).toBe("revealing");
        expect(onTimeout).toHaveBeenCalledWith(expect.arrayContaining(["history", "never-completes"]));
        expect(errSpy).toHaveBeenCalledWith(expect.stringContaining("never-completes"));
        errSpy.mockRestore();
        dispose();
    });

    it("does not fire the timeout once every gate completed in time", () => {
        const onTimeout = vi.fn();
        const { value: r, dispose } = inRoot(() => createPaneReadiness({ revealTimeoutMs: 5000, onTimeout }));
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
        const { value: r, dispose } = inRoot(() => createPaneReadiness({ revealTimeoutMs: 1000, onTimeout }));
        r.gate("history");
        dispose();
        vi.advanceTimersByTime(5000);
        expect(onTimeout).not.toHaveBeenCalled();
    });
});
