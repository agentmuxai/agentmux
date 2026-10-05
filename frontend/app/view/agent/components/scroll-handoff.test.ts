// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    SKID_FLASH_MS,
    SKID_GESTURE_IDLE_MS,
    SKID_IDLE,
    SKID_NOTCHES,
    attachScrollHandoff,
    nextSkid,
    type ScrollHandoffOptions,
    type SkidInput,
    type WheelSkid,
} from "./scroll-handoff";

interface Geo {
    scrollTop: number;
    clientHeight: number;
    scrollHeight: number;
}

/** A capped box with stubbed geometry. */
function makeBox(g: Geo): HTMLElement {
    const box = document.createElement("div");
    Object.defineProperty(box, "scrollTop", { configurable: true, get: () => g.scrollTop });
    Object.defineProperty(box, "clientHeight", { configurable: true, get: () => g.clientHeight });
    Object.defineProperty(box, "scrollHeight", { configurable: true, get: () => g.scrollHeight });
    return box;
}

/** The pane's `.agent-document` with one or more boxes, and a bare row outside them. */
function setup(...geos: Geo[]) {
    return setupWith({}, ...geos);
}

function setupWith(opts: ScrollHandoffOptions, ...geos: Geo[]) {
    const pane = document.createElement("div");
    pane.className = "agent-document";
    let paneTop = 1000;
    Object.defineProperty(pane, "scrollTop", { configurable: true, get: () => paneTop, set: (v) => (paneTop = v) });
    const row = document.createElement("div");
    pane.appendChild(row);
    const boxes = geos.map((g) => {
        const box = makeBox(g);
        pane.appendChild(box);
        return box;
    });
    document.body.appendChild(pane);
    const detach = boxes.map((b) => attachScrollHandoff(b, opts));
    let clock = 0;
    /** One wheel event; `wheelDeltaY` set = a notched wheel, absent = continuous. */
    const wheel = (target: Element, deltaY: number, o: { notch?: boolean; wheelDeltaY?: number; at?: number; ctrlKey?: boolean } = {}) => {
        const e = new WheelEvent("wheel", { deltaY, ctrlKey: o.ctrlKey ?? false, bubbles: true, cancelable: true });
        const wd = o.wheelDeltaY ?? (o.notch === false ? undefined : -Math.sign(deltaY) * 120);
        if (wd !== undefined) Object.defineProperty(e, "wheelDeltaY", { value: wd });
        clock = o.at ?? clock + 16;
        Object.defineProperty(e, "timeStamp", { value: clock });
        target.dispatchEvent(e);
        return e.defaultPrevented;
    };
    /** The full skid: SKID_NOTCHES notches that move nothing. */
    const skid = (target: Element, deltaY: number) => {
        for (let i = 0; i < SKID_NOTCHES; i++) expect(wheel(target, deltaY)).toBe(false);
    };
    return { pane: () => paneTop, row, boxes, box: boxes[0], wheel, skid, detach: () => detach.forEach((d) => d()) };
}

const atBottom: Geo = { scrollTop: 300, clientHeight: 200, scrollHeight: 500 };
const atTop: Geo = { scrollTop: 0, clientHeight: 200, scrollHeight: 500 };
const midway: Geo = { scrollTop: 100, clientHeight: 200, scrollHeight: 500 };

afterEach(() => {
    document.body.innerHTML = "";
    vi.useRealTimers();
});

// SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §2 and
// SPEC_TOOL_PREVIEW_WHEEL_EDGE_SKID_2026_09_27.md §4.3.
const N = SKID_NOTCHES;

describe("attachScrollHandoff", () => {
    it("skids three notches (the tuned value; the other tests follow SKID_NOTCHES)", () => {
        expect(SKID_NOTCHES).toBe(3);
    });

    it("at the bottom, the first notches skid and the next go to the pane", () => {
        const s = setup(atBottom);
        s.skid(s.box, 120);
        expect(s.pane()).toBe(1000);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1120);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1240);
    });

    it("at the top, the first notches up skid and the next goes to the pane", () => {
        const s = setup(atTop);
        s.skid(s.box, -120);
        expect(s.pane()).toBe(1000);
        s.wheel(s.box, -120);
        expect(s.pane()).toBe(880);
    });

    it("lets the box scroll itself while it has room, and re-arms the skid when it does", () => {
        const g = { ...midway };
        const s = setup(g);
        expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.pane()).toBe(1000);
        g.scrollTop = 300; // that notch brought it to its bottom
        s.skid(s.box, 120);
        expect(s.pane()).toBe(1000);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
    });

    it("never touches Ctrl+wheel (that's zoom) or a horizontal scroll", () => {
        const s = setup(atBottom);
        expect(s.wheel(s.box, 120, { ctrlKey: true })).toBe(false);
        expect(s.wheel(s.box, 0)).toBe(false);
        s.skid(s.box, 120); // the skid is still whole
        expect(s.pane()).toBe(1000);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
    });

    it("a box whose content fits never skids by default", () => {
        const s = setup({ scrollTop: 0, clientHeight: 200, scrollHeight: 200 });
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1120);
    });

    it("a box that grows from fitting to overflowing skids on the next notch", () => {
        const g = { scrollTop: 0, clientHeight: 200, scrollHeight: 200 };
        const s = setup(g);
        s.wheel(s.box, 120); // fits: handed on
        expect(s.pane()).toBe(1120);
        g.scrollHeight = 500; // output grew; the preview pins itself to its bottom
        g.scrollTop = 300;
        s.skid(s.box, 120);
        expect(s.pane()).toBe(1120);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1240);
    });

    it("with skidWhenFits (tool previews), a box that fits skids in either direction", () => {
        const fits = { scrollTop: 0, clientHeight: 200, scrollHeight: 200 };
        const s = setupWith({ skidWhenFits: true }, fits);
        // a box with no overflow cancels the absorbed notches itself
        for (let i = 0; i < N; i++) expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1000);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(true);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
        for (let i = 0; i < N; i++) expect(s.wheel(s.box, -120)).toBe(true); // reversing is a new arrival
        expect(s.pane()).toBe(1120);
        s.wheel(s.box, -120);
        expect(s.pane()).toBe(1000);
    });

    it("a coalesced event skids the first notches and forwards the one beyond", () => {
        const s = setup(atBottom);
        expect(s.wheel(s.box, (N + 1) * 100, { wheelDeltaY: -(N + 1) * 120 })).toBe(true);
        expect(s.pane()).toBe(1100);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(true);
    });

    it("a coalesced event can finish a skid that a single notch started", () => {
        const s = setup(atBottom);
        s.wheel(s.box, 120);
        expect(s.wheel(s.box, N * 100, { wheelDeltaY: -N * 120 })).toBe(true);
        expect(s.pane()).toBe(1100);
    });

    it("reversing direction re-arms: the box scrolls back first", () => {
        const s = setup(atBottom);
        s.skid(s.box, 120);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
        expect(s.wheel(s.box, -120)).toBe(false); // the box has room upward: native
        expect(s.pane()).toBe(1120);
    });

    it("leaving the box for the pane and coming back is a new arrival", () => {
        const s = setup(atBottom);
        s.skid(s.box, 120);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
        s.wheel(s.row, 120); // over the pane: native, resets the skid
        s.skid(s.box, 120);
        expect(s.pane()).toBe(1120);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1240);
    });

    it("another box arriving under the pointer gets its own skid", () => {
        const s = setup(atBottom, atBottom);
        s.skid(s.boxes[0], 120);
        s.wheel(s.boxes[0], 120);
        expect(s.pane()).toBe(1120);
        s.skid(s.boxes[1], 120);
        expect(s.pane()).toBe(1120);
        s.wheel(s.boxes[1], 120);
        expect(s.pane()).toBe(1240);
    });

    it("a trackpad gesture at the edge is absorbed until it pauses", () => {
        const s = setup(atBottom);
        s.wheel(s.box, 30, { notch: false, at: 1000 });
        s.wheel(s.box, 30, { notch: false, at: 1016 });
        s.wheel(s.box, 30, { notch: false, at: 1200 }); // momentum, still under the idle gap
        expect(s.pane()).toBe(1000);
        s.wheel(s.box, 30, { notch: false, at: 1200 + SKID_GESTURE_IDLE_MS }); // a new swipe
        expect(s.pane()).toBe(1030);
    });

    it("shows a line on the edge it is pinned against for the whole skid, then clears", () => {
        vi.useFakeTimers();
        const s = setup(atBottom);
        expect(s.box.classList.contains("scroll-handoff-box")).toBe(true);
        s.wheel(s.box, 120);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(true);
        vi.advanceTimersByTime(SKID_FLASH_MS - 50);
        s.wheel(s.box, 120); // the second notch keeps it on
        vi.advanceTimersByTime(SKID_FLASH_MS - 50);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(true);
        vi.advanceTimersByTime(50);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(false);
    });

    it("does nothing once detached", () => {
        const s = setup(atBottom);
        s.detach();
        for (let i = 0; i <= SKID_NOTCHES; i++) expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.pane()).toBe(1000);
        expect(s.box.classList.contains("scroll-handoff-box")).toBe(false);
    });
});

describe("nextSkid", () => {
    const box = {};
    const other = {};
    const input = (o: Partial<SkidInput> = {}): SkidInput => ({
        box,
        dir: 1,
        deltaY: 100,
        canSkid: true,
        atEdge: true,
        notches: 1,
        timeStamp: 0,
        ...o,
    });
    let s: WheelSkid;
    const step = (o: Partial<SkidInput> = {}) => {
        const r = nextSkid(s, input(o));
        s = r.state;
        return r.action;
    };
    beforeEach(() => {
        s = SKID_IDLE;
    });

    it("arrival at the edge: absorb SKID_NOTCHES notches, then forward", () => {
        for (let i = 0; i < N; i++) expect(step()).toEqual({ kind: "absorb" });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
    });

    it("off the edge is native and starts the skid over", () => {
        step();
        expect(step({ atEdge: false })).toEqual({ kind: "native" });
        for (let i = 0; i < N; i++) expect(step()).toEqual({ kind: "absorb" });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
    });

    it("a different box or direction is an arrival", () => {
        for (let i = 0; i <= N; i++) step();
        expect(step({ box: other })).toEqual({ kind: "absorb" });
        expect(step({ box: other, dir: -1, deltaY: -100 })).toEqual({ kind: "absorb" });
        expect(s.absorbed).toBe(1);
    });

    it("forwards the notches beyond the skid, and says it absorbed some", () => {
        expect(step({ notches: N + 1, deltaY: (N + 1) * 100 })).toEqual({ kind: "forward", deltaY: 100, absorbed: true });
        expect(s.phase).toBe("spent");
        expect(step({ notches: 2, deltaY: 200 })).toEqual({ kind: "forward", deltaY: 200 });
    });

    it("continuous input: absorbed under the idle gap, forwarded after it", () => {
        expect(step({ notches: null, timeStamp: 0 })).toEqual({ kind: "absorb" });
        expect(step({ notches: null, timeStamp: SKID_GESTURE_IDLE_MS - 1 })).toEqual({ kind: "absorb" });
        expect(step({ notches: null, timeStamp: 2 * SKID_GESTURE_IDLE_MS })).toEqual({ kind: "forward", deltaY: 100 });
    });

    it("a box that doesn't take part forwards at once", () => {
        expect(step({ canSkid: false })).toEqual({ kind: "forward", deltaY: 100 });
    });

    it("a box that starts overflowing after forwarding still skids", () => {
        expect(step({ canSkid: false })).toEqual({ kind: "forward", deltaY: 100 });
        for (let i = 0; i < N; i++) expect(step()).toEqual({ kind: "absorb" });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
    });
});
