// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { afterTwoFrames } from "./after-two-frames";

describe("afterTwoFrames", () => {
    let frames: Array<() => void>;

    beforeEach(() => {
        frames = [];
        vi.stubGlobal("requestAnimationFrame", (cb: () => void) => frames.push(cb));
    });
    afterEach(() => vi.unstubAllGlobals());

    // Each call to `paint` is one frame: it runs the callbacks queued before it, and not the ones they queue.
    const paint = () => frames.splice(0).forEach((cb) => cb());

    it("does not run at once, nor after one frame", () => {
        const run = vi.fn();
        afterTwoFrames(run);
        expect(run).not.toHaveBeenCalled();
        paint();
        expect(run).not.toHaveBeenCalled();
    });

    it("runs in the second frame, once, with no argument", () => {
        const run = vi.fn();
        afterTwoFrames(run);
        paint();
        paint();
        expect(run).toHaveBeenCalledTimes(1);
        expect(run).toHaveBeenCalledWith();
        paint();
        expect(run).toHaveBeenCalledTimes(1);
    });
});
