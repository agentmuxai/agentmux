// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    createSelectionRunner,
    createRequestedTabs,
    LAYOUT_POLL_MS,
    type ResolveOutcome,
    type SelectionDeps,
    type VerifyOutcome,
} from "./selection-effects";
import { DEADLINES_MS } from "./selection-machine";
import { createSelectionTrace } from "./selection-trace";

/** A fake window: every async step waits for the test to complete it, so the orders real timing
 *  produces once in a hundred are forced here. */
function fake() {
    const log: string[] = [];
    const resolves: Array<{ blockId: string; done: (o: ResolveOutcome) => void }> = [];
    const switches: Array<() => void> = [];
    const handoffs: Array<(found: boolean) => void> = [];
    const layout = new Set<string>();
    let verifyOutcomes: VerifyOutcome[] = [];
    const notices: Array<{ blockId: string; reason: string }> = [];
    const deps: SelectionDeps = {
        resolve: (intent) =>
            new Promise((done) => {
                log.push(`resolve:${intent.blockId}`);
                resolves.push({ blockId: intent.blockId, done });
            }),
        switchTab: (tabId) =>
            new Promise((done) => {
                log.push(`switchTab:${tabId}`);
                switches.push(done);
            }),
        layoutHas: (tabId, blockId) => layout.has(`${tabId}/${blockId}`),
        show: (tabId, blockId) => void log.push(`show:${tabId}/${blockId}`),
        focus: (tabId, blockId, caret) => void log.push(`focus:${tabId}/${blockId}${caret ? "+caret" : ""}`),
        settle: () => Promise.resolve(),
        verify: (tabId, blockId) => {
            log.push(`verify:${tabId}/${blockId}`);
            return verifyOutcomes.shift() ?? { ok: true };
        },
        handoff: (blockId) =>
            new Promise((done) => {
                log.push(`handoff:${blockId}`);
                handoffs.push(done);
            }),
        flash: (blockId) => void log.push(`flash:${blockId}`),
        notify: (intent, reason) => void notices.push({ blockId: intent.blockId, reason }),
        setTimer: (fn, ms) => setTimeout(fn, ms),
        clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
        now: () => Date.now(),
    };
    return {
        deps,
        log,
        resolves,
        switches,
        handoffs,
        layout,
        notices,
        verifyWith: (...o: VerifyOutcome[]) => (verifyOutcomes = o),
    };
}

const flush = async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
};

describe("the selection runner", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("shows a pane in another tab: switch, wait for its layout, show, focus, verify, flash", async () => {
        const f = fake();
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const result = runner.select({ blockId: "b1", source: "swarm" });
        f.resolves[0].done({ where: "here", tabId: "t2", active: false });
        await flush();
        f.switches[0]();
        await flush();
        expect(runner.state().kind).toBe("awaitingLayout");
        // The layout is not there yet: the runner keeps looking until it is.
        await vi.advanceTimersByTimeAsync(LAYOUT_POLL_MS * 3);
        expect(f.log).not.toContain("show:t2/b1");
        f.layout.add("t2/b1");
        await vi.advanceTimersByTimeAsync(LAYOUT_POLL_MS);
        await flush();
        expect(await result).toEqual({ ok: true });
        expect(f.log).toEqual(["resolve:b1", "switchTab:t2", "show:t2/b1", "focus:t2/b1+caret", "verify:t2/b1", "flash:b1"]);
        expect(runner.state().kind).toBe("settled");
    });

    it("A then B: B wins even when A's answers arrive last", async () => {
        const f = fake();
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        f.layout.add("t1/a");
        f.layout.add("t1/b");
        const a = runner.select({ blockId: "a" });
        const b = runner.select({ blockId: "b" });
        // B's resolve answers first, then A's.
        f.resolves[1].done({ where: "here", tabId: "t1", active: true });
        await flush();
        f.resolves[0].done({ where: "here", tabId: "t1", active: true });
        await flush();
        expect(await a).toEqual({ ok: false, reason: "superseded" });
        expect(await b).toEqual({ ok: true });
        expect(f.log.filter((l) => l.startsWith("show:") || l.startsWith("flash:"))).toEqual(["show:t1/b", "flash:b"]);
    });

    it("what the older request's tab switch does after the newer one started is dropped", async () => {
        const f = fake();
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const a = runner.select({ blockId: "a" });
        f.resolves[0].done({ where: "here", tabId: "t1", active: false });
        await flush();
        const b = runner.select({ blockId: "b" });
        f.layout.add("t1/a");
        f.switches[0](); // A's tab switch lands after B began
        await flush();
        expect(f.log).not.toContain("show:t1/a");
        f.resolves[1].done({ where: "none" });
        await flush();
        expect(await a).toEqual({ ok: false, reason: "superseded" });
        expect(await b).toMatchObject({ ok: false, reason: "not-found" });
        expect(f.notices).toEqual([{ blockId: "b", reason: "not-found" }]);
    });

    it("a layout that never arrives fails at its deadline with a notice, not silently", async () => {
        const f = fake();
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const result = runner.select({ blockId: "b1" });
        f.resolves[0].done({ where: "here", tabId: "t1", active: true });
        await flush();
        await vi.advanceTimersByTimeAsync(DEADLINES_MS.awaitingLayout! + 10);
        expect(await result).toMatchObject({ ok: false, reason: "layout-timeout" });
        expect(f.notices).toEqual([{ blockId: "b1", reason: "layout-timeout" }]);
        // No stray timer is left running.
        expect(vi.getTimerCount()).toBe(0);
    });

    it("a resolve that never answers fails at its deadline", async () => {
        const f = fake();
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const result = runner.select({ blockId: "b1" });
        await vi.advanceTimersByTimeAsync(DEADLINES_MS.resolving! + 10);
        expect(await result).toMatchObject({ ok: false, reason: "resolve-timeout" });
    });

    it("a mismatch shows the pane once more; a second one fails with what disagreed", async () => {
        const f = fake();
        f.layout.add("t1/b1");
        f.verifyWith({ ok: false, detail: "the pane is not the focused one" }, { ok: false, detail: "another pane is magnified" });
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const result = runner.select({ blockId: "b1" });
        f.resolves[0].done({ where: "here", tabId: "t1", active: true });
        await flush();
        expect(await result).toEqual({ ok: false, reason: "mismatch", detail: "another pane is magnified" });
        expect(f.log.filter((l) => l.startsWith("show:")).length).toBe(2);
        expect(f.log).not.toContain("flash:b1");
    });

    it("a mismatch that the retry repairs settles and flashes", async () => {
        const f = fake();
        f.layout.add("t1/b1");
        f.verifyWith({ ok: false, detail: "x" }, { ok: true });
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const result = runner.select({ blockId: "b1" });
        f.resolves[0].done({ where: "here", tabId: "t1", active: true });
        await flush();
        expect(await result).toEqual({ ok: true });
        expect(f.log).toContain("flash:b1");
    });

    it("a block that left its layout while it was verified fails as gone", async () => {
        const f = fake();
        f.layout.add("t1/b1");
        f.verifyWith({ gone: true });
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const result = runner.select({ blockId: "b1" });
        f.resolves[0].done({ where: "here", tabId: "t1", active: true });
        await flush();
        expect(await result).toMatchObject({ ok: false, reason: "gone" });
    });

    it("hands a block of another window to srv, and settles on its answer", async () => {
        const f = fake();
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const result = runner.select({ blockId: "far" });
        f.resolves[0].done({ where: "other" });
        await flush();
        f.handoffs[0](true);
        expect(await result).toEqual({ ok: true });
        expect(f.log).not.toContain("flash:far");
        const missing = runner.select({ blockId: "gone" });
        f.resolves[1].done({ where: "other" });
        await flush();
        f.handoffs[1](false);
        expect(await missing).toMatchObject({ ok: false, reason: "not-found" });
    });

    it("an effect that throws fails the request as an error instead of hanging", async () => {
        const f = fake();
        f.layout.add("t1/b1");
        f.deps.show = () => {
            throw new Error("the pane left its layout");
        };
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const result = runner.select({ blockId: "b1" });
        f.resolves[0].done({ where: "here", tabId: "t1", active: true });
        await flush();
        expect(await result).toEqual({ ok: false, reason: "error", detail: "the pane left its layout" });
    });

    it("the user selecting something else ends the request without a notice", async () => {
        const f = fake();
        const runner = createSelectionRunner(f.deps, createSelectionTrace(Date.now, () => {}));
        const result = runner.select({ blockId: "b1" });
        f.resolves[0].done({ where: "here", tabId: "t2", active: false });
        await flush();
        runner.userMoved();
        f.switches[0]();
        await flush();
        expect(await result).toEqual({ ok: false, reason: "superseded" });
        expect(f.notices).toEqual([]);
        expect(f.log).not.toContain("show:t2/b1");
        expect(runner.state().kind).toBe("idle");
    });

    it("the caret is skipped when the caller asks, and the trace records each step and the failure", async () => {
        const f = fake();
        const lines: string[] = [];
        const trace = createSelectionTrace(Date.now, (l) => lines.push(l));
        const runner = createSelectionRunner(f.deps, trace);
        f.layout.add("t1/b1");
        const done = runner.select({ blockId: "b1", source: "tower", focusCaret: false });
        f.resolves[0].done({ where: "here", tabId: "t1", active: true });
        await flush();
        await done;
        expect(f.log).toContain("focus:t1/b1");
        const entries = trace.entries().map((e) => e.line);
        expect(entries[0]).toMatch(/^\[select\] id=1 tower blk=b1 idle->resolving \+0ms$/);
        expect(entries.at(-1)).toMatch(/verifying->settled/);
        const failed = runner.select({ blockId: "x" });
        f.resolves[1].done({ where: "none" });
        await flush();
        await failed;
        expect(lines.some((l) => /resolving->failed .*not-found/.test(l))).toBe(true);
    });
});

describe("the tabs the runner asked for", () => {
    it("tells a late commit of its own switch from the user switching tab, and forgets it after a while", () => {
        let t = 1000;
        const tabs = createRequestedTabs(() => t, 5000);
        expect(tabs.isOurs("t2")).toBe(false);
        tabs.asked("t2");
        expect(tabs.isOurs("t2")).toBe(true);
        expect(tabs.isOurs("t9")).toBe(false);
        // t2 is the committed tab, yet a switch to t3 may still land: t2 cannot be relied on to stay.
        tabs.asked("t3");
        expect(tabs.pendingElsewhere("t2")).toBe(true);
        expect(tabs.pendingElsewhere("t3")).toBe(true);
        expect(createRequestedTabs(() => t).pendingElsewhere("t2")).toBe(false);
        t += 4999;
        expect(tabs.isOurs("t2")).toBe(true);
        t += 2;
        expect(tabs.isOurs("t2")).toBe(false);
        expect(tabs.pendingElsewhere("t2")).toBe(false);
    });
});
