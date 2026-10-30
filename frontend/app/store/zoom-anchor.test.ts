// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The timing of zoom that keeps the content under the pointer in place: when an anchor is taken, when
 * it is put back, and when the putting back stops. The pane's own maths is tested with its adapter.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { captureZoomAnchor, GESTURE_MS, SETTLE_FRAMES, useZoomAnchor, type ZoomAnchorAdapter } from "./zoom-anchor";

const P = { clientX: 100, clientY: 200 };

interface Pane {
    adapter: { capture: ReturnType<typeof vi.fn>; restore: ReturnType<typeof vi.fn> };
    setZoom: (z: number) => void;
    setFollowing: (f: boolean) => void;
    scroller: HTMLElement;
    dispose: () => void;
}

/** A pane on `zoom`; `restores` is what its successive restore passes answer (the last one repeats). */
function pane(blockId: string, restores: boolean[] = [true]): Pane {
    const scroller = document.createElement("div");
    document.body.appendChild(scroller);
    let i = 0;
    const adapter = {
        capture: vi.fn(() => ({ at: "held" })),
        restore: vi.fn(() => restores[Math.min(i++, restores.length - 1)]),
    };
    let out!: Pane;
    createRoot((dispose) => {
        const [zoom, setZoom] = createSignal(1);
        const [following, setFollowing] = createSignal(false);
        useZoomAnchor({ blockId, zoom, scroller: () => scroller, following, adapter: adapter as ZoomAnchorAdapter });
        out = { adapter, setZoom, setFollowing, scroller, dispose };
    });
    return out;
}

const frame = () => vi.advanceTimersByTime(17);

beforeEach(() => {
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "performance"] });
});
afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
});

describe("captureZoomAnchor", () => {
    it("takes the anchor from the pane before the change", () => {
        const p = pane("a");
        captureZoomAnchor("a", P, 0);
        expect(p.adapter.capture).toHaveBeenCalledWith(P);
        p.dispose();
    });

    it("steps of one gesture keep the first anchor: close in time, the pointer in the same place", () => {
        const p = pane("a");
        captureZoomAnchor("a", P, 0);
        captureZoomAnchor("a", { clientX: 102, clientY: 199 }, GESTURE_MS - 50);
        captureZoomAnchor("a", P, GESTURE_MS - 50 + GESTURE_MS - 50);
        expect(p.adapter.capture).toHaveBeenCalledTimes(1);
        p.dispose();
    });

    it("a pause, or a pointer that moved, is a new anchor", () => {
        const p = pane("a");
        captureZoomAnchor("a", P, 0);
        captureZoomAnchor("a", P, GESTURE_MS + 1);
        expect(p.adapter.capture).toHaveBeenCalledTimes(2);
        captureZoomAnchor("a", { clientX: 160, clientY: 200 }, GESTURE_MS + 2);
        expect(p.adapter.capture).toHaveBeenCalledTimes(3);
        p.dispose();
    });

    it("does nothing for a pane that did not opt in", () => {
        expect(() => captureZoomAnchor("nobody", P, 0)).not.toThrow();
    });
});

describe("restore after a zoom", () => {
    it("puts the anchor back as soon as the zoom changes, then checks it on later frames until two passes agree", () => {
        const p = pane("a", [false, true, true]);
        captureZoomAnchor("a", P, 0);
        p.setZoom(1.1);
        expect(p.adapter.restore).toHaveBeenCalledTimes(1);
        expect(p.adapter.restore).toHaveBeenCalledWith({ at: "held" });
        frame();
        frame();
        expect(p.adapter.restore).toHaveBeenCalledTimes(3);
        frame();
        frame();
        expect(p.adapter.restore).toHaveBeenCalledTimes(3); // settled: no more passes
        p.dispose();
    });

    it("gives up after a bounded number of frames when the position keeps moving", () => {
        const p = pane("a", [false]);
        captureZoomAnchor("a", P, 0);
        p.setZoom(1.1);
        for (let i = 0; i < SETTLE_FRAMES + 5; i++) frame();
        expect(p.adapter.restore).toHaveBeenCalledTimes(SETTLE_FRAMES);
        p.dispose();
    });

    it("restores nothing for a zoom that was not asked for here (no anchor was taken)", () => {
        const p = pane("a");
        p.setZoom(1.3);
        expect(p.adapter.restore).not.toHaveBeenCalled();
        p.dispose();
    });

    it("restores once per capture: a later change with no new capture is left alone", () => {
        const p = pane("a");
        captureZoomAnchor("a", P, 0);
        p.setZoom(1.1);
        frame();
        frame();
        const passes = p.adapter.restore.mock.calls.length;
        p.setZoom(1.2);
        expect(p.adapter.restore).toHaveBeenCalledTimes(passes);
        p.dispose();
    });

    it("a pane that does not hold an anchor (following, or nothing under the point) is not restored", () => {
        const p = pane("a");
        p.adapter.capture.mockReturnValue(null);
        captureZoomAnchor("a", P, 0);
        p.setZoom(1.1);
        expect(p.adapter.restore).not.toHaveBeenCalled();
        p.dispose();
    });

    it("stops when the pane starts following during the settle", () => {
        const p = pane("a", [false]);
        captureZoomAnchor("a", P, 0);
        p.setZoom(1.1);
        expect(p.adapter.restore).toHaveBeenCalledTimes(1);
        p.setFollowing(true);
        frame();
        frame();
        expect(p.adapter.restore).toHaveBeenCalledTimes(1);
        p.dispose();
    });

    it("stops when the reader scrolls, but not for a Ctrl+wheel (more zoom)", () => {
        const p = pane("a", [false]);
        captureZoomAnchor("a", P, 0);
        p.setZoom(1.1);
        p.scroller.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: -1 }));
        frame();
        expect(p.adapter.restore).toHaveBeenCalledTimes(2);
        p.scroller.dispatchEvent(new WheelEvent("wheel", { deltaY: 40 }));
        frame();
        frame();
        expect(p.adapter.restore).toHaveBeenCalledTimes(2);
        p.dispose();
    });

    it("stops on a scroll key or a press on the scrollbar", () => {
        const p = pane("a", [false]);
        captureZoomAnchor("a", P, 0);
        p.setZoom(1.1);
        p.scroller.dispatchEvent(new KeyboardEvent("keydown", { key: "PageDown" }));
        frame();
        expect(p.adapter.restore).toHaveBeenCalledTimes(1);

        const q = pane("b", [false]);
        captureZoomAnchor("b", P, 0);
        q.setZoom(1.1);
        q.scroller.dispatchEvent(new Event("pointerdown"));
        frame();
        expect(q.adapter.restore).toHaveBeenCalledTimes(1);
        p.dispose();
        q.dispose();
    });

    it("a second step during the settle starts the settle over from the same anchor", () => {
        const p = pane("a", [false, false, true, true]);
        captureZoomAnchor("a", P, 0);
        p.setZoom(1.1);
        frame();
        captureZoomAnchor("a", P, 30); // same gesture
        p.setZoom(1.2);
        expect(p.adapter.capture).toHaveBeenCalledTimes(1);
        expect(p.adapter.restore).toHaveBeenLastCalledWith({ at: "held" });
        p.dispose();
    });

    it("a pane that is gone is forgotten", () => {
        const p = pane("a");
        p.dispose();
        captureZoomAnchor("a", P, 0);
        expect(p.adapter.capture).not.toHaveBeenCalled();
    });
});
