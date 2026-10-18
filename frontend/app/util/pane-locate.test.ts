// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The locate flash's parts: the bus, the start-rate limit, the readiness gate
 * that waits for a pane to be on screen and still, and the overlay.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

// Whether the user is in this window (the host's view, not just document focus).
let userInWindow = true;
vi.mock("@/app/store/input-owner", () => ({ userInThisWindow: () => userInWindow }));

import {
    emitPaneLocate,
    emitPaneLocatePlay,
    LOCATE_HOLD_MS,
    LOCATE_MIN_GAP_MS,
    LOCATE_RISE_MS,
    LOCATE_STABLE_FRAMES,
    LOCATE_TOTAL_MS,
    locateKeyframes,
    locateSlotWaitMs,
    onPaneLocate,
    onPaneLocatePlay,
    recordLocateStart,
    samplePane,
    showLocateFlash,
    whenPaneReady,
    type PaneSample,
} from "./pane-locate";

describe("the bus", () => {
    it("tells every subscriber, requests and plays apart, and stops after unsubscribe", () => {
        const asked = vi.fn();
        const played = vi.fn();
        const offAsk = onPaneLocate(asked);
        const offPlay = onPaneLocatePlay(played);
        emitPaneLocate("a");
        emitPaneLocatePlay("b");
        expect(asked.mock.calls).toEqual([["a"]]);
        expect(played.mock.calls).toEqual([["b"]]);
        offAsk();
        offPlay();
        emitPaneLocate("c");
        emitPaneLocatePlay("c");
        expect(asked).toHaveBeenCalledTimes(1);
        expect(played).toHaveBeenCalledTimes(1);
    });

    it("a listener that throws does not stop the others", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const after = vi.fn();
        const off1 = onPaneLocate(() => {
            throw new Error("boom");
        });
        const off2 = onPaneLocate(after);
        emitPaneLocate("a");
        expect(after).toHaveBeenCalledWith("a");
        off1();
        off2();
        warn.mockRestore();
    });
});

describe("the start rate", () => {
    it("says how long a flash must wait for the previous one's gap, so three start a second at most", () => {
        const t0 = 1_000_000;
        recordLocateStart(t0);
        expect(locateSlotWaitMs(t0)).toBe(LOCATE_MIN_GAP_MS);
        expect(locateSlotWaitMs(t0 + 100)).toBe(LOCATE_MIN_GAP_MS - 100);
        expect(locateSlotWaitMs(t0 + LOCATE_MIN_GAP_MS)).toBe(0);
        expect(locateSlotWaitMs(t0 + 5000)).toBe(0);
        expect(LOCATE_MIN_GAP_MS * 3).toBeGreaterThan(1000);
    });
});

describe("samplePane", () => {
    const el = (checkVisibility = () => true): any => ({
        checkVisibility,
        getBoundingClientRect: () => ({ left: 1.4, top: 2, width: 100, height: 50 }),
    });
    const setDocument = (over: { visibilityState?: string; hasFocus?: boolean }) => {
        vi.stubGlobal("document", { visibilityState: over.visibilityState ?? "visible" });
        userInWindow = over.hasFocus ?? true;
    };
    afterEach(() => {
        vi.unstubAllGlobals();
        userInWindow = true;
    });

    it("reports the rounded box of a painted pane in the window the user is in", () => {
        setDocument({});
        expect(samplePane(el(), true)).toEqual({ ok: true, box: "1,2,100,50" });
    });

    it("is not ready while the window is hidden, not in front, or the pane is not painted or sized", () => {
        setDocument({ visibilityState: "hidden" });
        expect(samplePane(el(), true).ok).toBe(false);
        // Visible but still behind another window, as while it is being raised.
        setDocument({ hasFocus: false });
        expect(samplePane(el(), true).ok).toBe(false);
        setDocument({});
        expect(samplePane(el(() => false), true).ok).toBe(false);
        expect(samplePane(el(), false).ok).toBe(false);
        expect(samplePane(undefined, true).ok).toBe(false);
        const empty = { ...el(), getBoundingClientRect: () => ({ left: 0, top: 0, width: 0, height: 0 }) };
        expect(samplePane(empty, true)).toEqual({ ok: false, box: "" });
    });
});

/** Frames and a timer the test steps by hand. */
function fakeClock() {
    let frames: (() => void)[] = [];
    let timer: { cb: () => void; ms: number } | null = null;
    return {
        opts: (timeoutMs = 1500) => ({
            timeoutMs,
            requestFrame: (cb: () => void) => frames.push(cb),
            cancelFrame: () => {
                frames = [];
            },
            setTimer: (cb: () => void, ms: number) => {
                timer = { cb, ms };
                return 1;
            },
            clearTimer: () => {
                timer = null;
            },
        }),
        frame() {
            const run = frames;
            frames = [];
            run.forEach((f) => f());
        },
        fireTimer() {
            timer?.cb();
        },
        get pendingFrames() {
            return frames.length;
        },
        get timerMs() {
            return timer?.ms;
        },
    };
}

const at = (box: string, ok = true): PaneSample => ({ ok, box });

describe("whenPaneReady", () => {
    it("is ready once the pane has been on screen and unmoved for the stable frames", async () => {
        const clock = fakeClock();
        const gate = whenPaneReady(() => at("0,0,100,100"), clock.opts());
        const seen = vi.fn();
        void gate.ready.then(seen);
        // The first frame only records the box; then one more unchanged frame per count.
        for (let i = 0; i <= LOCATE_STABLE_FRAMES; i++) {
            await Promise.resolve();
            expect(seen).not.toHaveBeenCalled();
            clock.frame();
        }
        expect(await gate.ready).toBe(true);
    });

    it("starts over when the box moves, as a pane does while a layout settles", async () => {
        const clock = fakeClock();
        const boxes = ["0,0,50,50", "0,0,50,50", "0,0,100,100", "0,0,100,100", "0,0,100,100"];
        let i = 0;
        const gate = whenPaneReady(() => at(boxes[Math.min(i++, boxes.length - 1)]), clock.opts());
        let frames = 0;
        let ready = false;
        void gate.ready.then(() => (ready = true));
        while (!ready && frames < 20) {
            clock.frame();
            frames++;
            await Promise.resolve();
        }
        expect(ready).toBe(true);
        // Stable on the 50x50 box after 3 samples would have been 3 frames; the move cost more.
        expect(frames).toBeGreaterThan(LOCATE_STABLE_FRAMES + 1);
    });

    it("never fires while the pane is hidden, focus is elsewhere, or it has no box", async () => {
        const clock = fakeClock();
        let sample = at("1,1,1,1", false);
        const gate = whenPaneReady(() => sample, clock.opts());
        let ready = false;
        void gate.ready.then((r) => (ready = r));
        for (let i = 0; i < 10; i++) clock.frame();
        sample = at("");
        for (let i = 0; i < 10; i++) clock.frame();
        await Promise.resolve();
        expect(ready).toBe(false);
        expect(clock.pendingFrames).toBe(1);
        // It was waiting, not dead: it fires once the pane can be seen.
        sample = at("1,1,1,1");
        for (let i = 0; i < LOCATE_STABLE_FRAMES + 1; i++) clock.frame();
        expect(await gate.ready).toBe(true);
    });

    it("gives up at the timeout, and stops looking", async () => {
        const clock = fakeClock();
        const gate = whenPaneReady(() => at("", false), clock.opts(3000));
        expect(clock.timerMs).toBe(3000);
        clock.fireTimer();
        expect(await gate.ready).toBe(false);
        expect(clock.pendingFrames).toBe(0);
    });

    it("cancel ends it as not ready", async () => {
        const clock = fakeClock();
        const gate = whenPaneReady(() => at("0,0,9,9"), clock.opts());
        gate.cancel();
        expect(await gate.ready).toBe(false);
        expect(clock.pendingFrames).toBe(0);
    });
});

describe("locateKeyframes", () => {
    it("rises, holds, then fades to nothing over the whole flash", () => {
        const kf = locateKeyframes();
        expect(kf.map((k) => [k.offset, k.opacity])).toEqual([
            [0, 0],
            [LOCATE_RISE_MS / LOCATE_TOTAL_MS, 1],
            [(LOCATE_RISE_MS + LOCATE_HOLD_MS) / LOCATE_TOTAL_MS, 1],
            [1, 0],
        ]);
        // Quick up, slow down: the fade is most of the time.
        expect(LOCATE_TOTAL_MS - LOCATE_RISE_MS - LOCATE_HOLD_MS).toBeGreaterThan(5 * LOCATE_RISE_MS);
    });
});

describe("showLocateFlash", () => {
    function fakeHost(animate?: (kf: unknown, o: unknown) => any) {
        const el: any = {
            className: "",
            attrs: {} as Record<string, string>,
            style: { props: {} as Record<string, string>, setProperty(k: string, v: string) { this.props[k] = v; } },
            setAttribute(k: string, v: string) { this.attrs[k] = v; },
            removed: false,
            remove() { this.removed = true; },
            ...(animate ? { animate } : {}),
        };
        const host: any = {
            children: [] as any[],
            ownerDocument: { createElement: () => el },
            appendChild(c: any) { this.children.push(c); },
        };
        return { host, el };
    }

    it("adds the overlay, plays the envelope once, and removes it when it ends", () => {
        const anim: any = {};
        const animate = vi.fn(() => anim);
        const { host, el } = fakeHost(animate);
        showLocateFlash(host, { light: false, reduced: false });
        expect(host.children).toEqual([el]);
        expect(el.className).toBe("pane-locate-flash");
        expect(el.attrs["aria-hidden"]).toBe("true");
        expect(el.style.props["--pane-locate-wash"]).toBe("16%");
        expect(animate).toHaveBeenCalledWith(locateKeyframes(), { duration: LOCATE_TOTAL_MS });
        expect(el.removed).toBe(false);
        anim.onfinish();
        expect(el.removed).toBe(true);
    });

    it("a light theme gets the lighter wash; the motion policy drops the ring", () => {
        const { host, el } = fakeHost(() => ({}));
        showLocateFlash(host, { light: true, reduced: true });
        expect(el.style.props["--pane-locate-wash"]).toBe("12%");
        expect(el.className).toBe("pane-locate-flash pane-locate-flash--reduced");
    });

    it("the returned stop cancels the animation and removes the overlay at once, and is safe to repeat", () => {
        const cancel = vi.fn();
        const anim: any = { cancel };
        const { host, el } = fakeHost(() => anim);
        const stop = showLocateFlash(host, { light: false, reduced: false });
        stop();
        stop();
        expect(cancel).toHaveBeenCalledTimes(1);
        expect(el.removed).toBe(true);
    });

    it("leaves nothing behind where animations aren't supported", () => {
        const { host, el } = fakeHost();
        showLocateFlash(host, { light: false, reduced: false });
        expect(el.removed).toBe(true);
    });
});

afterEach(() => vi.restoreAllMocks());
