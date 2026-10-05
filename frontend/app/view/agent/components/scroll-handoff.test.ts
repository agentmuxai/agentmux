// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    SKID_FLASH_MS,
    SKID_GESTURE_IDLE_MS,
    SKID_IDLE,
    attachScrollHandoff,
    nextSkid,
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
    const detach = boxes.map((b) => attachScrollHandoff(b));
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
    return { pane: () => paneTop, row, boxes, box: boxes[0], wheel, detach: () => detach.forEach((d) => d()) };
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
describe("attachScrollHandoff", () => {
    it("at the bottom, the first notch skids and the next goes to the pane", () => {
        const s = setup(atBottom);
        expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.pane()).toBe(1000);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1120);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1240);
    });

    it("at the top, the first notch up skids and the next goes to the pane", () => {
        const s = setup(atTop);
        s.wheel(s.box, -120);
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
        s.wheel(s.box, 120); // skid
        expect(s.pane()).toBe(1000);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
    });

    it("never touches Ctrl+wheel (that's zoom) or a horizontal scroll", () => {
        const s = setup(atBottom);
        expect(s.wheel(s.box, 120, { ctrlKey: true })).toBe(false);
        expect(s.wheel(s.box, 0)).toBe(false);
        s.wheel(s.box, 120); // still the first notch at the edge: skids
        expect(s.pane()).toBe(1000);
    });

    it("a box whose content fits never skids", () => {
        const s = setup({ scrollTop: 0, clientHeight: 200, scrollHeight: 200 });
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1120);
    });

    it("a coalesced three-notch event skids one notch and forwards the other two", () => {
        const s = setup(atBottom);
        expect(s.wheel(s.box, 300, { wheelDeltaY: -360 })).toBe(true);
        expect(s.pane()).toBe(1200);
    });

    it("reversing direction re-arms: the box scrolls back first", () => {
        const g = { ...atBottom };
        const s = setup(g);
        s.wheel(s.box, 120);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
        expect(s.wheel(s.box, -120)).toBe(false); // the box has room upward: native
        expect(s.pane()).toBe(1120);
    });

    it("leaving the box for the pane and coming back is a new arrival", () => {
        const s = setup(atBottom);
        s.wheel(s.box, 120);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
        s.wheel(s.row, 120); // over the pane: native, resets the skid
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1240);
    });

    it("another box arriving under the pointer gets its own skid", () => {
        const s = setup(atBottom, atBottom);
        s.wheel(s.boxes[0], 120);
        s.wheel(s.boxes[0], 120);
        expect(s.pane()).toBe(1120);
        s.wheel(s.boxes[1], 120);
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

    it("flashes the edge it is pinned against while it skids", () => {
        vi.useFakeTimers();
        const s = setup(atBottom);
        expect(s.box.classList.contains("scroll-handoff-box")).toBe(true);
        s.wheel(s.box, 120);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(true);
        vi.advanceTimersByTime(SKID_FLASH_MS);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(false);
    });

    it("does nothing once detached", () => {
        const s = setup(atBottom);
        s.detach();
        expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.wheel(s.box, 120)).toBe(false);
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
        overflows: true,
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

    it("arrival at the edge: absorb, then forward", () => {
        expect(step()).toEqual({ kind: "absorb" });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
    });

    it("off the edge is native and re-arms", () => {
        step();
        step();
        expect(step({ atEdge: false })).toEqual({ kind: "native" });
        expect(step()).toEqual({ kind: "absorb" });
    });

    it("a different box or direction is an arrival", () => {
        step();
        step();
        expect(step({ box: other })).toEqual({ kind: "absorb" });
        expect(step({ box: other, dir: -1, deltaY: -100 })).toEqual({ kind: "absorb" });
    });

    it("forwards the notches beyond the first", () => {
        expect(step({ notches: 3, deltaY: 300 })).toEqual({ kind: "forward", deltaY: 200 });
        expect(s.phase).toBe("spent");
    });

    it("continuous input: absorbed under the idle gap, forwarded after it", () => {
        expect(step({ notches: null, timeStamp: 0 })).toEqual({ kind: "absorb" });
        expect(step({ notches: null, timeStamp: SKID_GESTURE_IDLE_MS - 1 })).toEqual({ kind: "absorb" });
        expect(step({ notches: null, timeStamp: 2 * SKID_GESTURE_IDLE_MS })).toEqual({ kind: "forward", deltaY: 100 });
    });

    it("a box that doesn't overflow forwards at once", () => {
        expect(step({ overflows: false })).toEqual({ kind: "forward", deltaY: 100 });
    });
});
