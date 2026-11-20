// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A derived value that is recomputed at most once per animation frame.
 *
 * `createMemo` recomputes at once, inside the write that changed one of its sources, so a burst of
 * writes (a broadcast starting a turn on a dozen agents: every status, progress and block update
 * lands in the same second) runs an expensive derivation once per write. Here the first write
 * after a computation only marks the value stale and asks for a frame; every later write before
 * that frame costs nothing. In the frame the value is recomputed once, from whatever the sources
 * hold by then, so the final value is the one the per-write memo would have ended on.
 *
 * Sources are tracked by Solid's `createReaction`: `compute` runs under tracking each time, and a
 * change to anything it read schedules the next run instead of running it.
 *
 * The first value is computed at once, so a reader never sees a placeholder. A stale value is
 * shown until the frame; the delay is under one frame.
 */

import { createReaction, createSignal, getOwner, onCleanup, type Accessor } from "solid-js";

/** Where "the next frame" comes from. The browser's by default; a test passes its own. */
export interface FrameScheduler {
    request(run: () => void): unknown;
    cancel(handle: unknown): void;
}

export const browserFrames: FrameScheduler = {
    request: (run) => requestAnimationFrame(() => run()),
    cancel: (handle) => cancelAnimationFrame(handle as number),
};

export function createFrameMemo<T>(compute: () => T, frames: FrameScheduler = browserFrames): Accessor<T> {
    let pending: unknown;
    let disposed = false;
    const track = createReaction(() => {
        if (disposed || pending !== undefined) return;
        pending = frames.request(refresh);
    });
    const run = (): T => {
        let value!: T;
        track(() => {
            value = compute();
        });
        return value;
    };
    const [value, setValue] = createSignal<T>(run());
    function refresh(): void {
        pending = undefined;
        if (disposed) return;
        const next = run();
        setValue(() => next);
    }
    if (getOwner()) {
        onCleanup(() => {
            disposed = true;
            if (pending !== undefined) frames.cancel(pending);
            pending = undefined;
        });
    }
    return value;
}
