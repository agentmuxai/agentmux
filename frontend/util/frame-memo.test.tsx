// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createRoot, createSignal, type Accessor } from "solid-js";
import { describe, expect, it } from "vitest";
import { createFrameMemo, type FrameScheduler } from "./frame-memo";

/** Frames the test fires by hand. */
function manualFrames() {
    let next = 1;
    const queued = new Map<number, () => void>();
    const frames: FrameScheduler = {
        request: (run) => {
            const id = next++;
            queued.set(id, run);
            return id;
        },
        cancel: (id) => void queued.delete(id as number),
    };
    return {
        frames,
        queued: () => queued.size,
        frame: () => {
            const runs = [...queued.values()];
            queued.clear();
            runs.forEach((run) => run());
        },
    };
}

/**
 * Builds in a root and hands back what the test needs. The test then writes outside the root's
 * body: writes inside it are held until the body returns, so nothing would have fired yet.
 */
function inRoot<T>(build: () => T): T & { dispose: () => void } {
    let dispose!: () => void;
    const built = createRoot((d) => {
        dispose = d;
        return build();
    });
    return { ...built, dispose };
}

describe("createFrameMemo", () => {
    it("computes the first value at once", () => {
        const { frames } = manualFrames();
        const { value, dispose } = inRoot(() => ({ value: createFrameMemo(() => 7, frames) }));
        expect(value()).toBe(7);
        dispose();
    });

    it("runs the derivation once per frame however many writes a burst makes", () => {
        const { frames, frame, queued } = manualFrames();
        const state = { runs: 0 };
        const { panes, total, dispose } = inRoot(() => {
            const panes = Array.from({ length: 14 }, () => createSignal(0));
            const total = createFrameMemo(() => {
                state.runs++;
                return panes.reduce((sum, [get]) => sum + get(), 0);
            }, frames);
            return { panes, total };
        });
        expect(state.runs).toBe(1);

        for (let round = 1; round <= 5; round++) panes.forEach(([, set]) => set(round));
        expect(state.runs).toBe(1);
        expect(queued()).toBe(1);
        expect(total()).toBe(0);

        frame();
        expect(state.runs).toBe(2);
        expect(total()).toBe(14 * 5);
        dispose();
    });

    it("keeps following its sources after a frame", () => {
        const { frames, frame } = manualFrames();
        const state = { runs: 0 };
        const { set, value, dispose } = inRoot(() => {
            const [get, set] = createSignal(1);
            const value = createFrameMemo(() => (state.runs++, get() * 10), frames);
            return { set, value };
        });
        set(2);
        frame();
        expect(value()).toBe(20);
        set(3);
        set(4);
        frame();
        expect(value()).toBe(40);
        expect(state.runs).toBe(3);
        dispose();
    });

    it("asks for no frame while nothing changed", () => {
        const { frames, queued, frame } = manualFrames();
        const { value, dispose } = inRoot(() => {
            const [get] = createSignal(1);
            return { value: createFrameMemo(() => get(), frames) };
        });
        expect(queued()).toBe(0);
        frame();
        expect(value()).toBe(1);
        dispose();
    });

    it("tells its readers once per frame", () => {
        const { frames, frame } = manualFrames();
        const seen: number[] = [];
        const { set, dispose } = inRoot(() => {
            const [get, set] = createSignal(0);
            const value: Accessor<number> = createFrameMemo(() => get(), frames);
            createEffect(() => seen.push(value()));
            return { set };
        });
        expect(seen).toEqual([0]);
        set(1);
        set(2);
        expect(seen).toEqual([0]);
        frame();
        expect(seen).toEqual([0, 2]);
        dispose();
    });

    it("stops asking for frames once its owner is gone", () => {
        const { frames, queued, frame } = manualFrames();
        const state = { runs: 0 };
        const { set, dispose } = inRoot(() => {
            const [get, set] = createSignal(0);
            createFrameMemo(() => (state.runs++, get()), frames);
            return { set };
        });
        set(1);
        expect(queued()).toBe(1);
        dispose();
        expect(queued()).toBe(0);
        frame();
        set(2);
        expect(queued()).toBe(0);
        expect(state.runs).toBe(1);
    });
});
