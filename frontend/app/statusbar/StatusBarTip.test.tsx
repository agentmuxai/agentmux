// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * StatusBarTip — the delegated `[data-tip]` hover balloon.
 *
 * These cover the disposal race that made a leaked `autoUpdate` throw
 * `Cannot read properties of null (reading 'getBoundingClientRect')` on
 * every subsequent scroll/resize. `TipBalloon` starts floating-ui's
 * `autoUpdate` from inside a `requestAnimationFrame`, so a tip that is
 * dismissed before that frame runs disposes the component while
 * `cleanupAutoUpdate` is still null — cleanup has nothing to cancel, the
 * frame then starts an observer nothing owns, and its anchor accessor
 * reads a `props.target` that is now null.
 *
 * Observed live: ~500 throws/second for ten seconds (2026-09-22 09:15Z,
 * a fast cursor sweep across the status bar), 4,718 in that hour.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { autoUpdate } from "@floating-ui/dom";
import { StatusBarTip } from "./StatusBarTip";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
}));

vi.mock("@/app/platform/pane-overlay", () => ({
    usePaneOverlay: vi.fn(),
}));

vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(() =>
        Promise.resolve({ style: { position: "fixed", left: "0px", top: "0px" } }),
    ),
}));

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

/** Hand-driven rAF so the "dismissed before the frame ran" window is
 *  reachable deterministically, and so `cancelAnimationFrame` is observable. */
let pendingFrames: Array<{ id: number; cb: FrameRequestCallback }> = [];
let nextFrameId = 1;
const flushFrames = (): void => {
    const due = pendingFrames;
    pendingFrames = [];
    for (const frame of due) frame.cb(0);
};

function mountTip(): HTMLElement {
    const { container } = render(() => (
        <>
            <div class="status-bar">
                <button type="button" data-tip="Disk usage">
                    disk
                </button>
            </div>
            <StatusBarTip />
        </>
    ));
    return container.querySelector<HTMLElement>("[data-tip]")!;
}

describe("StatusBarTip", () => {
    beforeEach(() => {
        pendingFrames = [];
        nextFrameId = 1;
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            const id = nextFrameId++;
            pendingFrames.push({ id, cb });
            return id;
        });
        vi.stubGlobal("cancelAnimationFrame", (id: number) => {
            pendingFrames = pendingFrames.filter((frame) => frame.id !== id);
        });
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        vi.mocked(autoUpdate).mockClear();
    });

    it("starts autoUpdate once the balloon's frame runs", async () => {
        const anchor = mountTip();
        fireEvent.mouseOver(anchor);
        await tick();
        expect(autoUpdate).not.toHaveBeenCalled(); // still waiting on the frame
        flushFrames();
        expect(autoUpdate).toHaveBeenCalledTimes(1);
    });

    it("stops autoUpdate when the tip is dismissed", async () => {
        const stop = vi.fn();
        vi.mocked(autoUpdate).mockReturnValueOnce(stop);
        const anchor = mountTip();
        fireEvent.mouseOver(anchor);
        await tick();
        flushFrames();
        expect(autoUpdate).toHaveBeenCalledTimes(1);

        fireEvent.mouseOut(anchor, { relatedTarget: document.body });
        await tick();
        expect(stop).toHaveBeenCalledTimes(1);
    });

    // The regression. Dismissing inside the same frame the balloon mounted in
    // left `cleanupAutoUpdate` null at disposal, so the pending frame went on
    // to start an observer with no owner left to stop it. Every later
    // scroll/resize then called back into a disposed component.
    it("does not start autoUpdate when dismissed before its frame runs", async () => {
        const anchor = mountTip();
        fireEvent.mouseOver(anchor);
        await tick();
        fireEvent.mouseOut(anchor, { relatedTarget: document.body });
        await tick();

        flushFrames(); // the frame queued by the now-disposed balloon

        expect(autoUpdate).not.toHaveBeenCalled();
    });

    // Even a balloon that did start autoUpdate can have an in-flight position
    // update racing disposal. The anchor must stay readable rather than
    // dereferencing a `props.target` that the parent's `<Show>` has nulled.
    it("keeps the anchor readable after the tip is dismissed", async () => {
        const anchor = mountTip();
        anchor.getBoundingClientRect = () => new DOMRect(4, 8, 16, 32);

        fireEvent.mouseOver(anchor);
        await tick();
        flushFrames();

        const reference = vi.mocked(autoUpdate).mock.calls[0]![0] as {
            getBoundingClientRect: () => DOMRect;
        };
        expect(reference.getBoundingClientRect().x).toBe(4);

        fireEvent.mouseOut(anchor, { relatedTarget: document.body });
        await tick();

        expect(() => reference.getBoundingClientRect()).not.toThrow();
    });
});
