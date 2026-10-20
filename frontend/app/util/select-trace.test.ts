// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import { CLICK_GRACE_MS, createSelectTrace, selectTrace, traced } from "./select-trace";

function harness() {
    let t = 1_000_000;
    const logs: string[] = [];
    const timers: { at: number; fn: () => void; id: number }[] = [];
    let nextId = 1;
    const trace = createSelectTrace({
        now: () => t,
        log: (l) => logs.push(l),
        setTimer: (fn, ms) => {
            const id = nextId++;
            timers.push({ at: t + ms, fn, id });
            return id;
        },
        clearTimer: (h) => {
            const i = timers.findIndex((x) => x.id === h);
            if (i >= 0) timers.splice(i, 1);
        },
    });
    const advance = (ms: number) => {
        t += ms;
        for (const x of [...timers].filter((x) => x.at <= t).sort((a, b) => a.at - b.at)) {
            timers.splice(timers.indexOf(x), 1);
            x.fn();
        }
    };
    return { trace, logs, advance };
}

describe("rebuild causes", () => {
    it("blames a rebuild on the signals that moved since the last one", () => {
        const { trace } = harness();
        trace.cause("status");
        trace.cause("status");
        trace.cause("progress");
        trace.rebuilt();
        trace.rebuilt();
        const c = trace.counters();
        expect(c.rebuilds).toBe(2);
        expect(c.rebuildsByCause).toEqual({ status: 1, progress: 1, other: 1 });
    });

    it("writes one summary line when the minute ends, even if nothing else happens, and starts the next window empty", () => {
        const { trace, logs, advance } = harness();
        trace.cause("poll-same");
        trace.rebuilt();
        trace.rebuilt();
        advance(59_000);
        expect(logs).toEqual([]);
        advance(1_000);
        expect(logs).toHaveLength(1);
        expect(logs[0]).toContain("rebuilds=2");
        expect(logs[0]).toContain("poll-same=1");
        expect(trace.counters().rebuilds).toBe(0);

        advance(120_000);
        expect(logs).toHaveLength(1);
        trace.rebuilt();
        advance(60_000);
        expect(logs).toHaveLength(2);
    });
});

describe("traced setters", () => {
    it("record the cause before readers recompute, and skip a write that changes nothing", () => {
        const [get, set] = createSignal<number[]>([1]);
        const write = traced("status", [get, set]);
        createRoot((dispose) => {
            createMemo(() => {
                get();
                selectTrace.rebuilt();
            });
            const initial = selectTrace.counters().rebuildsByCause;
            expect(initial).toEqual({ other: 1 });

            write(get());
            write((prev) => prev);
            expect(selectTrace.counters().rebuildsByCause).toEqual({ other: 1 });

            write([2]);
            expect(selectTrace.counters().rebuildsByCause).toEqual({ other: 1, status: 1 });
            dispose();
        });
    });
});

describe("pressed rows whose click never arrives", () => {
    it("is not lost when the click follows the release", () => {
        const { trace, logs, advance } = harness();
        trace.pressed("block-aaaaaaaa", { isConnected: true });
        trace.released();
        trace.clicked("block-aaaaaaaa", { isConnected: true });
        advance(CLICK_GRACE_MS * 2);
        expect(trace.counters().lostClicks).toBe(0);
        expect(logs).toEqual([]);
    });

    it("counts a press with no click, and says the row had left the document", () => {
        const { trace, logs, advance } = harness();
        const row = { isConnected: true };
        trace.pressed("block-aaaaaaaa", row);
        advance(90);
        row.isConnected = false;
        trace.released();
        advance(CLICK_GRACE_MS);
        const c = trace.counters();
        expect(c.presses).toBe(1);
        expect(c.lostClicks).toBe(1);
        expect(c.lostClicksRowGone).toBe(1);
        expect(logs[0]).toContain("block-aa");
        expect(logs[0]).toContain("rebuilt under the pointer");
    });

    it("a click that landed on the list, or on another row, does not save the press", () => {
        const { trace, advance } = harness();
        trace.pressed("block-aaaaaaaa", { isConnected: false });
        trace.released();
        trace.clicked(null, null);
        trace.clicked("block-bbbbbbbb", { isConnected: true });
        trace.clicked("block-aaaaaaaa", { isConnected: false });
        advance(CLICK_GRACE_MS);
        expect(trace.counters().lostClicks).toBe(1);
    });

    it("a cancelled pointer sequence is not a lost click", () => {
        const { trace, advance } = harness();
        trace.pressed("block-aaaaaaaa", { isConnected: true });
        trace.cancelled();
        advance(CLICK_GRACE_MS * 2);
        expect(trace.counters().lostClicks).toBe(0);
    });

    it("a release with no press on a row is ignored", () => {
        const { trace, advance } = harness();
        trace.released();
        advance(CLICK_GRACE_MS * 2);
        expect(trace.counters().lostClicks).toBe(0);
    });

    it("a new press replaces one still waiting", () => {
        const { trace, advance } = harness();
        trace.pressed("a", { isConnected: true });
        trace.released();
        trace.pressed("b", { isConnected: true });
        advance(CLICK_GRACE_MS * 2);
        expect(trace.counters().lostClicks).toBe(0);
    });
});

describe("reveals", () => {
    it("logs a miss and an overlap, not an ordinary quick reveal", () => {
        const { trace, logs, advance } = harness();
        const first = trace.revealStarted("swarm", "block-1");
        advance(40);
        first(true);
        expect(logs).toEqual([]);

        const a = trace.revealStarted("swarm", "block-2");
        const b = trace.revealStarted("tower", "block-3");
        advance(30);
        b(false);
        a(true);
        expect(logs.some((l) => l.includes("src=tower") && l.includes("NOT-FOUND") && l.includes("OVERLAP"))).toBe(true);
        expect(trace.counters().overlappingReveals).toBe(1);
    });

    it("an outcome reported twice counts once", () => {
        const { trace } = harness();
        const done = trace.revealStarted("swarm", "block-1");
        done(true);
        done(false);
        expect(trace.entries().filter((e) => e.line.startsWith("[select]"))).toHaveLength(1);
    });

    it("keeps only the last 100 entries", () => {
        const { trace } = harness();
        for (let i = 0; i < 130; i++) trace.revealStarted("swarm", `b${i}`)(true);
        expect(trace.entries()).toHaveLength(100);
    });
});
