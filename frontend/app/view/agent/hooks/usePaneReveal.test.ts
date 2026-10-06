// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { historyStillLoading } from "../components/HistoryLoadingRow";
import { AGENT_REVEAL_TIMEOUT_MS, usePaneReveal } from "./usePaneReveal";

/** Manual settle + frame queues, so each step of the paint wait is explicit. */
function manualDeps() {
    let settle: (() => void) | undefined;
    const frames: Array<() => void> = [];
    return {
        deps: {
            scheduleOnSettle: (cb: () => void) => {
                settle = cb;
                return () => (settle = undefined);
            },
            frame: (cb: () => void) => frames.push(cb),
            cancelFrame: () => {},
        },
        settle: () => settle?.(),
        nextFrame: () => frames.shift()?.(),
    };
}

function mount(launchPhase: { kind: string } | null = { kind: "ready" }) {
    const m = manualDeps();
    const [phase, setPhase] = createSignal<{ kind: string } | null>(launchPhase);
    const root = createRoot((dispose) => {
        const reveal = usePaneReveal({ blockId: "b1", agentName: () => "Ada", deps: m.deps });
        reveal.connectLaunchStatus({ launchPhase: phase, authStatus: () => "authenticated" });
        return { reveal, dispose };
    });
    return { ...root, ...m, setPhase };
}

describe("usePaneReveal", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("stays covered until the history has painted: settle, then two frames", () => {
        const r = mount();
        expect(r.reveal.readiness.phase()).toBe("assembling");
        r.reveal.startPaintWait();
        r.settle();
        r.nextFrame();
        expect(r.reveal.readiness.phase()).toBe("assembling");
        r.nextFrame();
        expect(r.reveal.readiness.phase()).toBe("revealing");
        r.dispose();
    });

    it("goes live 220ms after it starts revealing (the fade's own duration)", () => {
        const r = mount();
        r.reveal.startPaintWait();
        r.settle();
        r.nextFrame();
        r.nextFrame();
        vi.advanceTimersByTime(219);
        expect(r.reveal.readiness.phase()).toBe("revealing");
        vi.advanceTimersByTime(1);
        expect(r.reveal.readiness.phase()).toBe("live");
        r.dispose();
    });

    it("holds the reveal while the launch flow can still pop the auth panel in", () => {
        const r = mount({ kind: "checking-auth" });
        r.reveal.startPaintWait();
        r.settle();
        r.nextFrame();
        r.nextFrame();
        expect(r.reveal.readiness.phase()).toBe("assembling");
        expect(r.reveal.readiness.pendingGates()).toEqual(["auth"]);
        r.setPhase({ kind: "ready" });
        expect(r.reveal.readiness.phase()).toBe("revealing");
        r.dispose();
    });

    it("stops holding for the launch flow at the reveal bound, so a pane can't stay covered", () => {
        const r = mount(null);
        r.reveal.startPaintWait();
        r.settle();
        r.nextFrame();
        r.nextFrame();
        expect(r.reveal.readiness.phase()).toBe("assembling");
        vi.advanceTimersByTime(AGENT_REVEAL_TIMEOUT_MS);
        expect(r.reveal.readiness.phase()).toBe("revealing");
        r.dispose();
    });

    it("shows the pane at the reveal bound while the history is still loading, and says so", () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        const r = mount();
        vi.advanceTimersByTime(AGENT_REVEAL_TIMEOUT_MS - 1);
        expect(r.reveal.readiness.phase()).toBe("assembling");
        expect(historyStillLoading(r.reveal.readiness)).toBe(false);
        vi.advanceTimersByTime(1);
        expect(r.reveal.readiness.phase()).toBe("revealing");
        expect(historyStillLoading(r.reveal.readiness)).toBe(true);
        // The history arrives and paints: the loading row goes.
        r.reveal.startPaintWait();
        r.settle();
        r.nextFrame();
        r.nextFrame();
        expect(historyStillLoading(r.reveal.readiness)).toBe(false);
        r.dispose();
    });
});
