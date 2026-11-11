// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import { deferWhileSizesHeld, holdPaneSizes, paneSizesHeld, resetPaneSizeHoldForTests } from "./pane-size-hold";

beforeEach(() => resetPaneSizeHoldForTests());

describe("the hold on pane sizes during a pane drag", () => {
    it("runs a resize callback at once when nothing is held", () => {
        const run = vi.fn();
        deferWhileSizesHeld(run)();
        expect(run).toHaveBeenCalledTimes(1);
    });

    it("keeps a callback back while sizes are held, and runs it once when the hold ends, however often it was called", () => {
        const model = {};
        const run = vi.fn();
        const onResize = deferWhileSizesHeld(run);
        holdPaneSizes(model, true);
        expect(paneSizesHeld()).toBe(true);
        onResize();
        onResize();
        onResize();
        expect(run).not.toHaveBeenCalled();
        holdPaneSizes(model, false);
        expect(paneSizesHeld()).toBe(false);
        expect(run).toHaveBeenCalledTimes(1);
        // Nothing is left waiting for the next hold.
        holdPaneSizes(model, true);
        holdPaneSizes(model, false);
        expect(run).toHaveBeenCalledTimes(1);
    });

    it("stays held until every holder lets go", () => {
        const [a, b] = [{}, {}];
        const run = vi.fn();
        holdPaneSizes(a, true);
        holdPaneSizes(b, true);
        deferWhileSizesHeld(run)();
        holdPaneSizes(a, false);
        expect(run).not.toHaveBeenCalled();
        holdPaneSizes(b, false);
        expect(run).toHaveBeenCalledTimes(1);
    });

    it("forgets a waiting callback that is cancelled (its pane went away)", () => {
        const model = {};
        const run = vi.fn();
        const onResize = deferWhileSizesHeld(run);
        holdPaneSizes(model, true);
        onResize();
        onResize.cancel();
        holdPaneSizes(model, false);
        expect(run).not.toHaveBeenCalled();
    });

    it("a release by one who never held changes nothing", () => {
        const run = vi.fn();
        holdPaneSizes({}, true);
        deferWhileSizesHeld(run)();
        holdPaneSizes({}, false);
        expect(paneSizesHeld()).toBe(true);
        expect(run).not.toHaveBeenCalled();
    });
});
