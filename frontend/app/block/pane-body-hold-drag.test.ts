// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BEHIND_FRAME_MS, DEFAULT_DRAG_REFRESH_MS, STARVED_ROUNDS, DRAG_PAUSE_MS, dragRefreshMs, dragRefreshTick, splitterDragActive, usePaneBodyHold } from "./pane-body-hold";

const KEY = "agentmux:pane-drag-refresh-ms";

/** A `.block-content` whose frame size the test can change, counting how often its width is measured. */
function frame(size: { w: number; h: number }) {
    const el = document.createElement("div");
    el.className = "block-content";
    const reads = { n: 0 };
    Object.defineProperty(el, "clientWidth", { get: () => (reads.n++, size.w) });
    Object.defineProperty(el, "clientHeight", { get: () => size.h });
    document.body.append(el);
    return { el, size, reads };
}

function mount(dragging: () => boolean, el: HTMLElement, view: string | undefined = "term") {
    let hold!: ReturnType<typeof usePaneBodyHold>;
    const dispose = createRoot((d) => {
        hold = usePaneBodyHold(dragging, () => el, () => view);
        return d;
    });
    return { hold, dispose };
}

describe("usePaneBodyHold during a drag", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        localStorage.clear();
    });
    afterEach(() => {
        vi.useRealTimers();
        document.body.replaceChildren();
        localStorage.clear();
    });

    it("holds the size at the start, and nothing before or after", () => {
        const f = frame({ w: 400, h: 300 });
        const [dragging, setDragging] = createSignal(false);
        const { hold, dispose } = mount(dragging, f.el);
        expect(hold()).toBeNull();
        setDragging(true);
        expect(hold()).toEqual({ width: 400, height: 300 });
        setDragging(false);
        expect(hold()).toBeNull();
        dispose();
    });

    it("lays the body out again at the frame's new size on each refresh, not on every pointer move", () => {
        const f = frame({ w: 400, h: 300 });
        const [dragging, setDragging] = createSignal(true);
        const { hold, dispose } = mount(dragging, f.el);
        f.size.w = 450; // the frame follows the pointer; the body does not, until the next refresh
        expect(hold()).toEqual({ width: 400, height: 300 });
        vi.advanceTimersByTime(DEFAULT_DRAG_REFRESH_MS);
        expect(hold()).toEqual({ width: 450, height: 300 });
        setDragging(false);
        dispose();
    });

    it("does not write a new hold when the frame's size has not changed", () => {
        const f = frame({ w: 400, h: 300 });
        const [dragging] = createSignal(true);
        const { hold, dispose } = mount(dragging, f.el);
        const first = hold();
        f.size.w = 400.4; // under a pixel: not a change
        vi.advanceTimersByTime(DEFAULT_DRAG_REFRESH_MS * 3);
        expect(hold()).toBe(first);
        dispose();
    });

    it("runs one clock for every held pane, and stops it with the last", () => {
        const a = frame({ w: 400, h: 300 });
        const b = frame({ w: 200, h: 300 });
        const [dragging, setDragging] = createSignal(true);
        const m1 = mount(dragging, a.el);
        const m2 = mount(dragging, b.el);
        expect(vi.getTimerCount()).toBe(1);
        setDragging(false);
        expect(vi.getTimerCount()).toBe(0);
        m1.dispose();
        m2.dispose();
    });


    describe("the catch-up clock", () => {
        const move = () => window.dispatchEvent(new Event("pointermove"));
        // A frame is 16 ms on the fake clock.
        const frames = (n: number) => vi.advanceTimersByTime(16 * n);

        it("waits while the pointer keeps moving, and catches up when it pauses", () => {
            const f = frame({ w: 400, h: 300 });
            const [dragging, setDragging] = createSignal(true);
            const { hold, dispose } = mount(dragging, f.el);
            f.size.w = 450;
            for (let t = 0; t < DEFAULT_DRAG_REFRESH_MS - 100; t += 16) {
                move();
                frames(1);
            }
            expect(hold()).toEqual({ width: 400, height: 300 });
            vi.advanceTimersByTime(DRAG_PAUSE_MS + 32);
            expect(hold()).toEqual({ width: 450, height: 300 });
            setDragging(false);
            dispose();
        });

        it("catches up at the latest after the pace even while the pointer keeps moving", () => {
            const f = frame({ w: 400, h: 300 });
            const [dragging, setDragging] = createSignal(true);
            const { hold, dispose } = mount(dragging, f.el);
            f.size.w = 450;
            for (let t = 0; t < DEFAULT_DRAG_REFRESH_MS + 64; t += 16) {
                move();
                frames(1);
            }
            expect(hold()).toEqual({ width: 450, height: 300 });
            setDragging(false);
            dispose();
        });

        it("lays out one body in a frame, not all of them in one", () => {
            const a = frame({ w: 400, h: 300 });
            const b = frame({ w: 200, h: 300 });
            const c = frame({ w: 300, h: 300 });
            const [dragging, setDragging] = createSignal(true);
            const holds = [mount(dragging, a.el), mount(dragging, b.el), mount(dragging, c.el)];
            for (const fr of [a, b, c]) fr.size.w += 50;
            vi.advanceTimersByTime(DRAG_PAUSE_MS);
            const changed = () => holds.filter((m, i) => m.hold()!.width !== [400, 200, 300][i]).length;
            let most = 0;
            for (let i = 0; i < 8; i++) {
                const before = changed();
                frames(1);
                most = Math.max(most, changed() - before);
            }
            expect(most).toBe(1);
            expect(changed()).toBe(3);
            setDragging(false);
            holds.forEach((m) => m.dispose());
        });

        it("does not lay out a body in a frame that began late: the page is behind", () => {
            // Every frame comes later than BEHIND_FRAME_MS after the one before it.
            const spy = vi
                .spyOn(window, "requestAnimationFrame")
                .mockImplementation((cb) => window.setTimeout(() => cb(performance.now()), BEHIND_FRAME_MS + 20) as unknown as number);
            const cancel = vi.spyOn(window, "cancelAnimationFrame").mockImplementation((id) => window.clearTimeout(id));
            const f = frame({ w: 400, h: 300 });
            const [dragging, setDragging] = createSignal(true);
            const { hold, dispose } = mount(dragging, f.el);
            f.size.w = 450;
            vi.advanceTimersByTime(DEFAULT_DRAG_REFRESH_MS * (STARVED_ROUNDS - 1));
            expect(hold()).toEqual({ width: 400, height: 300 });
            // ... but not for ever: a display that only ever draws late still catches up, at a bounded wait.
            vi.advanceTimersByTime(DEFAULT_DRAG_REFRESH_MS * 2);
            expect(hold()).toEqual({ width: 450, height: 300 });
            setDragging(false);
            dispose();
            spy.mockRestore();
            cancel.mockRestore();
        });

        it("ticks the browser panes' clock once a round, and says a drag is on only while it is", () => {
            const f = frame({ w: 400, h: 300 });
            const [dragging, setDragging] = createSignal(false);
            const { dispose } = mount(dragging, f.el);
            expect(splitterDragActive()).toBe(false);
            setDragging(true);
            expect(splitterDragActive()).toBe(true);
            const before = dragRefreshTick();
            f.size.w = 450;
            vi.advanceTimersByTime(DEFAULT_DRAG_REFRESH_MS);
            expect(dragRefreshTick()).toBeGreaterThan(before);
            setDragging(false);
            expect(splitterDragActive()).toBe(false);
            dispose();
        });
    });

    describe("with a ResizeObserver", () => {
        // The module keeps one observer for the page, made by the first pane that mounts with a ResizeObserver
        // to hand: every fake shares one registry, so the test can report sizes whichever test made it.
        const observed = new Set<Element>();
        let callback: ResizeObserverCallback | undefined;
        class FakeObserver {
            constructor(c: ResizeObserverCallback) {
                callback = c;
            }
            observe(el: Element) {
                observed.add(el);
            }
            unobserve(el: Element) {
                observed.delete(el);
            }
            disconnect() {}
        }
        const report = (el: Element, w: number, h: number) =>
            callback?.(
                [{ target: el, contentBoxSize: [{ inlineSize: w, blockSize: h }], contentRect: { width: w, height: h } } as unknown as ResizeObserverEntry],
                {} as ResizeObserver,
            );

        beforeEach(() => {
            vi.stubGlobal("ResizeObserver", FakeObserver);
        });
        afterEach(() => {
            vi.unstubAllGlobals();
            observed.clear();
        });

        it("holds the size the observer last reported, without asking the page to lay out", () => {
            const f = frame({ w: 1, h: 1 });
            const [dragging, setDragging] = createSignal(false);
            const { hold, dispose } = mount(dragging, f.el);
            expect(observed.has(f.el)).toBe(true);
            report(f.el, 640, 360);
            f.reads.n = 0;
            setDragging(true);
            expect(hold()).toEqual({ width: 640, height: 360 });
            report(f.el, 700, 360); // the frame follows the pointer; the body does not, until the next refresh
            expect(hold()).toEqual({ width: 640, height: 360 });
            vi.advanceTimersByTime(DEFAULT_DRAG_REFRESH_MS);
            expect(hold()).toEqual({ width: 700, height: 360 });
            expect(f.reads.n).toBe(0);
            setDragging(false);
            dispose();
            expect(observed.has(f.el)).toBe(false);
        });

        it("measures when the observer has not reported yet", () => {
            const f = frame({ w: 320, h: 200 });
            const [dragging] = createSignal(true);
            const { hold, dispose } = mount(dragging, f.el);
            expect(hold()).toEqual({ width: 320, height: 200 });
            dispose();
        });
    });

    it("repaints only on release when the refresh pace is 0", () => {
        localStorage.setItem(KEY, "0");
        const f = frame({ w: 400, h: 300 });
        const [dragging] = createSignal(true);
        const timersBefore = vi.getTimerCount(); // storage itself may hold a timer
        const { hold, dispose } = mount(dragging, f.el);
        expect(vi.getTimerCount()).toBe(timersBefore);
        f.size.w = 500;
        vi.advanceTimersByTime(5000);
        expect(hold()).toEqual({ width: 400, height: 300 });
        dispose();
    });

    it("never holds a browser pane", () => {
        const f = frame({ w: 400, h: 300 });
        const [dragging] = createSignal(true);
        const { hold, dispose } = mount(dragging, f.el, "browser");
        expect(hold()).toBeNull();
        dispose();
    });
});

describe("dragRefreshMs", () => {
    afterEach(() => localStorage.clear());

    it("is the default, or the stored pace, 0 included; a bad value is the default", () => {
        expect(dragRefreshMs()).toBe(DEFAULT_DRAG_REFRESH_MS);
        localStorage.setItem(KEY, "60");
        expect(dragRefreshMs()).toBe(60);
        localStorage.setItem(KEY, "0");
        expect(dragRefreshMs()).toBe(0);
        localStorage.setItem(KEY, "fast");
        expect(dragRefreshMs()).toBe(DEFAULT_DRAG_REFRESH_MS);
        localStorage.setItem(KEY, "-5");
        expect(dragRefreshMs()).toBe(DEFAULT_DRAG_REFRESH_MS);
    });
});
