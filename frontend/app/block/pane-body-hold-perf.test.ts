// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/** The `[perf] border-content` line: how far behind its frame a held body was while the border was dragged. */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_DRAG_REFRESH_MS, usePaneBodyHold } from "./pane-body-hold";

// A size observer the test drives: it reports when told to.
const observed = vi.hoisted(() => ({ report: null as null | ((el: Element) => void) }));
vi.stubGlobal(
    "ResizeObserver",
    class {
        constructor(cb: (entries: unknown[]) => void) {
            observed.report = (el) => cb([{ target: el, contentRect: { width: (el as HTMLElement).clientWidth, height: (el as HTMLElement).clientHeight } }]);
        }
        observe() {}
        unobserve() {}
        disconnect() {}
    }
);

function frame(size: { w: number; h: number }) {
    const el = document.createElement("div");
    Object.defineProperty(el, "clientWidth", { get: () => size.w });
    Object.defineProperty(el, "clientHeight", { get: () => size.h });
    document.body.append(el);
    return el;
}

function mount(dragging: () => boolean, el: HTMLElement, view: string) {
    return createRoot((d) => {
        usePaneBodyHold(dragging, () => el, () => view);
        return d;
    });
}

const move = () => window.dispatchEvent(new Event("pointermove"));
const lines = (warn: { mock: { calls: unknown[][] } }) => warn.mock.calls.map((c) => String(c[0])).filter((l) => l.includes("border-content"));

describe("[perf] border-content", () => {
    let warn: ReturnType<typeof vi.spyOn>;
    beforeEach(() => {
        vi.useFakeTimers();
        localStorage.clear();
        warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    });
    afterEach(() => {
        vi.useRealTimers();
        warn.mockRestore();
        document.body.replaceChildren();
    });

    it("reports a mid-drag catch-up by view kind, with how far behind the body was", () => {
        const size = { w: 400, h: 300 };
        const el = frame(size);
        const [dragging, setDragging] = createSignal(true);
        const dispose = mount(dragging, el, "term");
        for (let i = 0; i < 6; i++) move();
        size.w = 450;
        vi.advanceTimersByTime(100);
        observed.report!(el); // the frame changed size 100 ms before the catch-up
        vi.advanceTimersByTime(DEFAULT_DRAG_REFRESH_MS);
        setDragging(false);
        dispose();
        const out = lines(warn);
        expect(out).toHaveLength(1);
        expect(out[0]).toMatch(/^\[perf\] border-content moves=6 /);
        expect(out[0]).toMatch(/mid-drag catch-ups term\(n=1 p50=\d+ms worst=\d+ms\)/);
        expect(out[0]).toContain("behind at release 0");
    });

    it("reports a body still behind when the drag ended", () => {
        const size = { w: 400, h: 300 };
        const el = frame(size);
        const [dragging, setDragging] = createSignal(true);
        const dispose = mount(dragging, el, "agent");
        for (let i = 0; i < 6; i++) move();
        size.w = 500;
        observed.report!(el);
        vi.advanceTimersByTime(30); // no catch-up yet: the pointer is still moving and the pace has not run out
        setDragging(false);
        dispose();
        const out = lines(warn);
        expect(out).toHaveLength(1);
        expect(out[0]).toContain("mid-drag catch-ups none");
        expect(out[0]).toMatch(/behind at release 1 worst=\d+ms/);
    });

    it("logs nothing for a click on the border, with too few pointer moves for a drag", () => {
        const el = frame({ w: 400, h: 300 });
        const [dragging, setDragging] = createSignal(true);
        const dispose = mount(dragging, el, "term");
        move();
        setDragging(false);
        dispose();
        expect(lines(warn)).toEqual([]);
    });

    it("counts the size a body holds as the drag begins as no catch-up", () => {
        const el = frame({ w: 400, h: 300 });
        const [dragging, setDragging] = createSignal(false);
        const dispose = mount(dragging, el, "term");
        observed.report!(el); // reported before the drag: not behind by anything
        setDragging(true);
        for (let i = 0; i < 6; i++) move();
        setDragging(false);
        dispose();
        expect(lines(warn)[0]).toContain("mid-drag catch-ups none; behind at release 0");
    });

    it("takes a body whose frame went back to the size it holds as not behind", () => {
        const size = { w: 400, h: 300 };
        const el = frame(size);
        const [dragging, setDragging] = createSignal(true);
        const dispose = mount(dragging, el, "term");
        for (let i = 0; i < 6; i++) move();
        size.w = 450;
        observed.report!(el);
        size.w = 400; // back where it was before the next catch-up
        observed.report!(el);
        vi.advanceTimersByTime(DEFAULT_DRAG_REFRESH_MS + 100);
        setDragging(false);
        dispose();
        expect(lines(warn)[0]).toContain("mid-drag catch-ups none; behind at release 0");
    });

    it("counts a body that is never held (a browser pane) as behind by nothing", () => {
        const size = { w: 400, h: 300 };
        const el = frame(size);
        const [dragging, setDragging] = createSignal(true);
        const dispose = mount(dragging, el, "browser");
        for (let i = 0; i < 6; i++) move();
        size.w = 450;
        observed.report!(el);
        vi.advanceTimersByTime(DEFAULT_DRAG_REFRESH_MS + 100);
        setDragging(false);
        dispose();
        expect(lines(warn)[0]).toContain("mid-drag catch-ups none; behind at release 0");
    });
});
