// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { runInViewTransition } from "./view-transition";

const skipped = () => new DOMException("Transition was skipped. New ViewTransition started", "AbortError");
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function stubTransition(ready: Promise<void>, finished: Promise<void>) {
    const start = vi.fn((update: () => void) => {
        update();
        return { ready, finished, updateCallbackDone: Promise.resolve(), skipTransition: () => {} };
    });
    (document as unknown as { startViewTransition: unknown }).startViewTransition = start;
    return start;
}

describe("runInViewTransition", () => {
    afterEach(() => {
        delete (document as unknown as { startViewTransition?: unknown }).startViewTransition;
        vi.restoreAllMocks();
    });

    it("runs the update, inside the transition where there is one", () => {
        const start = stubTransition(Promise.resolve(), Promise.resolve());
        const update = vi.fn();
        runInViewTransition(update);
        expect(start).toHaveBeenCalledTimes(1);
        expect(update).toHaveBeenCalledTimes(1);
    });

    it("runs the update at once where the browser has no view transitions", () => {
        const update = vi.fn();
        runInViewTransition(update);
        expect(update).toHaveBeenCalledTimes(1);
    });

    it("a transition the next one skipped is not an unhandled rejection", async () => {
        // An unhandled rejection fails the run, so reaching the end is the assertion.
        const ready = Promise.reject(skipped());
        const finished = Promise.reject(skipped());
        const reject = vi.spyOn(Promise, "reject");
        stubTransition(ready, finished);
        runInViewTransition(() => {});
        await flush();
        expect(reject).not.toHaveBeenCalled();
    });

    it("any other rejection still surfaces", async () => {
        const boom = new Error("boom");
        const ready = Promise.reject(boom);
        const reject = vi.spyOn(Promise, "reject").mockImplementation(() => Promise.resolve() as never);
        stubTransition(ready, Promise.resolve());
        runInViewTransition(() => {});
        await flush();
        expect(reject).toHaveBeenCalledWith(boom);
    });
});
