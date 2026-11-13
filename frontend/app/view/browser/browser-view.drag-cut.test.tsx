// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A browser page is cut away behind its picture for an in-app drag, and the live reflow shows no drop until
 * the cut has landed (page-cut-gate.ts). "Landed" is the host's answer to the clip that carries the page's
 * hole, not a number of frames after the hole was asked for: where the host hides the whole page for a hole,
 * the clip waits for the page's freeze-frame first (pane-overlay.ts `flushClip`, use-freeze-frame.ts).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { pageCutsPending, resetPageCutGateForTests } from "@/app/drag/page-cut-gate";
import { makeTestHostApi } from "@/app/host/test-host";
import { __resetPaneRectRegistry, registerPaneRect } from "@/app/platform/pane-rect-registry";
import { BrowserDragCatcher } from "./browser-view";

const PAGE = { x: 100, y: 80, w: 600, h: 400 };
/** Long enough for a frame or two and the promise turns behind them. */
const settle = (ms = 40) => new Promise((r) => setTimeout(r, ms));

/** A host whose answers to a clip are held until the test gives them. */
function host() {
    const held: (() => void)[] = [];
    let holding = false;
    const setOverlayClip = vi.fn(() => (holding ? new Promise<void>((r) => held.push(r)) : Promise.resolve()));
    window.api = makeTestHostApi({ browserPanes: { setOverlayClip } as unknown as BrowserPaneHostApi });
    return {
        setOverlayClip,
        hold: () => (holding = true),
        answer: () => {
            holding = false;
            held.splice(0).forEach((r) => r());
        },
    };
}

/** The page's own view deferring the hide until its freeze-frame is painted, as use-freeze-frame.ts does. */
function deferHide() {
    const painted: (() => void)[] = [];
    const onClip = (e: Event) => {
        const detail = (e as CustomEvent).detail as { rects: unknown[]; wait: (p: Promise<unknown>) => void };
        if (detail.rects.length > 0) detail.wait(new Promise<void>((r) => painted.push(r)));
    };
    window.addEventListener("pane-overlay-clip-changed", onClip);
    return { paint: () => painted.splice(0).forEach((r) => r()), stop: () => window.removeEventListener("pane-overlay-clip-changed", onClip) };
}

let h: ReturnType<typeof host>;
beforeEach(() => {
    resetPageCutGateForTests();
    h = host();
});
afterEach(async () => {
    cleanup();
    h.answer();
    // The hole's release clears the clip: let that flush run before the next test.
    await settle();
    h.answer();
    await settle(10);
    __resetPaneRectRegistry();
});

describe("the cut of a browser page for a drag", () => {
    it("holds the reflow while the hide is deferred for the page's freeze-frame, and until the host has answered it", async () => {
        registerPaneRect("page", PAGE);
        const frame = deferHide();
        h.hold();
        render(() => <BrowserDragCatcher takeSnapshot={async () => null} pageRect={() => PAGE} />);
        expect(pageCutsPending()).toBe(true);
        // Far more than two frames: the hide has not been sent, so the page still stands where it was.
        await settle(120);
        expect(h.setOverlayClip).not.toHaveBeenCalled();
        expect(pageCutsPending()).toBe(true);
        // The freeze-frame is painted: the hide goes out, and the gate waits for the host's answer.
        frame.paint();
        await settle();
        expect(h.setOverlayClip).toHaveBeenCalledTimes(1);
        expect(pageCutsPending()).toBe(true);
        h.answer();
        await settle(10);
        expect(pageCutsPending()).toBe(false);
        frame.stop();
    });

    it("releases the reflow as soon as the host has answered when nothing defers the hide", async () => {
        registerPaneRect("page", PAGE);
        render(() => <BrowserDragCatcher takeSnapshot={async () => null} pageRect={() => PAGE} />);
        expect(pageCutsPending()).toBe(true);
        await settle();
        expect(h.setOverlayClip).toHaveBeenCalledTimes(1);
        expect(pageCutsPending()).toBe(false);
    });

    it("does not hold the reflow for a page that is not on screen: there is nothing to cut", async () => {
        render(() => <BrowserDragCatcher takeSnapshot={async () => null} pageRect={() => undefined} />);
        await settle(10);
        expect(pageCutsPending()).toBe(false);
        expect(h.setOverlayClip).not.toHaveBeenCalled();
    });

    it("stops holding when the drag ends before the cut has landed", async () => {
        registerPaneRect("page", PAGE);
        const frame = deferHide();
        const { unmount } = render(() => <BrowserDragCatcher takeSnapshot={async () => null} pageRect={() => PAGE} />);
        await settle();
        expect(pageCutsPending()).toBe(true);
        unmount();
        expect(pageCutsPending()).toBe(false);
        frame.paint();
        frame.stop();
    });
});
