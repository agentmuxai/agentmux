// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";

// The setting the skid follows, as the Settings pane would set it (undefined = unset).
const skidSetting = vi.hoisted(() => ({ value: undefined as unknown }));
vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: () => () => skidSetting.value }));
import {
    SKID_FLASH_MS,
    SKID_GESTURE_IDLE_MS,
    SKID_IDLE,
    SKID_NOTCHES_DEFAULT,
    SKID_NOTCHES_MAX,
    resolveSkidNotches,
    attachScrollHandoff,
    nextSkid,
    type SkidInput,
    type WheelSkid,
    previewBox,
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

/** The pane's `.agent-document` with one or more boxes, and a bare row outside them. Each box gets a copy of
 *  its geometry (`geos`, `geo` for the first), which a test moves as the browser would. */
function setup(...given: Geo[]) {
    const pane = document.createElement("div");
    pane.className = "agent-document";
    let paneTop = 1000;
    Object.defineProperty(pane, "scrollTop", { configurable: true, get: () => paneTop, set: (v) => (paneTop = v) });
    const row = document.createElement("div");
    pane.appendChild(row);
    const geos = given.map((g) => ({ ...g }));
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
    /** The wheel scrolls box `i` to its edge in `deltaY`'s direction: a notch the box takes itself, which
     *  brings it there. Only a box the wheel scrolled holds the wheel at its edge. */
    const toEdge = (deltaY: number, i = 0, o: { notch?: boolean } = {}) => {
        const g = geos[i];
        const edge = deltaY > 0 ? g.scrollHeight - g.clientHeight : 0;
        g.scrollTop = edge - Math.sign(deltaY) * 100;
        expect(wheel(boxes[i], deltaY, o)).toBe(false);
        g.scrollTop = edge;
    };
    /** The full skid: the setting's notches (2 unless a test sets it) that move nothing. */
    const skid = (target: Element, deltaY: number) => {
        for (let i = 0; i < resolveSkidNotches(skidSetting.value); i++) expect(wheel(target, deltaY)).toBe(false);
    };
    return { pane: () => paneTop, row, boxes, box: boxes[0], geos, geo: geos[0], wheel, toEdge, skid, detach: () => detach.forEach((d) => d()) };
}

const atBottom: Geo = { scrollTop: 300, clientHeight: 200, scrollHeight: 500 };
const atTop: Geo = { scrollTop: 0, clientHeight: 200, scrollHeight: 500 };
const midway: Geo = { scrollTop: 100, clientHeight: 200, scrollHeight: 500 };
const fitting: Geo = { scrollTop: 0, clientHeight: 200, scrollHeight: 200 };

afterEach(() => {
    skidSetting.value = undefined;
    document.body.innerHTML = "";
    vi.useRealTimers();
});

// SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §2 and
// SPEC_TOOL_PREVIEW_WHEEL_EDGE_SKID_2026_09_27.md §4.3, as amended in its §9.
const N = SKID_NOTCHES_DEFAULT;

describe("attachScrollHandoff", () => {
    it("skids two notches at an edge the wheel scrolled the box to, unless the setting says otherwise", () => {
        expect(SKID_NOTCHES_DEFAULT).toBe(2);
        const s = setup(midway);
        s.toEdge(120);
        expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1120);
    });

    it("follows agent:previewskidnotches, read at each wheel so a change needs no reopening", () => {
        skidSetting.value = 4;
        const s = setup(midway);
        s.toEdge(120);
        for (let i = 0; i < 4; i++) expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1120);
        // Scrolled to its edge again after the wheel left it: the new value applies to the open pane.
        s.wheel(s.row, 120);
        skidSetting.value = 1;
        s.toEdge(120);
        expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.wheel(s.box, 120)).toBe(true);
    });

    it("0 means no skid: the first notch at the edge goes to the pane, with no edge line", () => {
        skidSetting.value = 0;
        const s = setup(midway);
        s.toEdge(120);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1120);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(false);
    });

    it("0 still leaves a preview with room to scroll to its own wheel", () => {
        skidSetting.value = 0;
        const s = setup(midway);
        expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.pane()).toBe(1000);
    });

    it("a setting that is not a whole number from 0 up is the default, and a big one is held to the maximum", () => {
        for (const bad of [-1, 1.5, NaN, "3", null, undefined, true]) expect(resolveSkidNotches(bad)).toBe(SKID_NOTCHES_DEFAULT);
        expect(resolveSkidNotches(0)).toBe(0);
        expect(resolveSkidNotches(7)).toBe(7);
        expect(resolveSkidNotches(SKID_NOTCHES_MAX + 5)).toBe(SKID_NOTCHES_MAX);
    });

    it("at the bottom the wheel scrolled it to, the first notches skid and the next go to the pane", () => {
        const s = setup(midway);
        s.toEdge(120);
        s.skid(s.box, 120);
        expect(s.pane()).toBe(1000);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1120);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1240);
    });

    it("at the top the wheel scrolled it to, the first notches up skid and the next goes to the pane", () => {
        const s = setup(midway);
        s.toEdge(-120);
        s.skid(s.box, -120);
        expect(s.pane()).toBe(1000);
        s.wheel(s.box, -120);
        expect(s.pane()).toBe(880);
    });

    it("lets the box scroll itself while it has room, and holds the wheel at the edge that brings it to", () => {
        const s = setup(midway);
        expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.pane()).toBe(1000);
        s.geo.scrollTop = 300; // that notch brought it to its bottom
        s.skid(s.box, 120);
        expect(s.pane()).toBe(1000);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
    });

    // The dead spots of #47733: the pane scrolls with the pointer at rest, previews slide under it, and each
    // one that was at its edge that way used to swallow the skid's notches before the pane moved again.
    it("a box the wheel finds already at its edge hands every notch straight on", () => {
        const down = setup(atBottom);
        expect(down.wheel(down.box, 120)).toBe(true);
        expect(down.wheel(down.box, 120)).toBe(true);
        expect(down.pane()).toBe(1240);
        expect(down.box.classList.contains("scroll-handoff-skid--bottom")).toBe(false);
        const up = setup(atTop);
        expect(up.wheel(up.box, -120)).toBe(true);
        expect(up.pane()).toBe(880);
    });

    it("a box whose content fits hands every notch straight on, in both directions", () => {
        const s = setup(fitting);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1240);
        expect(s.wheel(s.box, -120)).toBe(true);
        expect(s.pane()).toBe(1120);
        expect(s.box.className).toBe("scroll-handoff-box");
    });

    it("a notch left to a box that did not move it is no scroll: the browser kept the gesture for the pane", () => {
        const s = setup(atTop);
        expect(s.wheel(s.box, 120)).toBe(false); // room below, so it is left to the box; the pane took it
        expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.wheel(s.box, -120)).toBe(true); // still at its top: nothing to hold the wheel for
        expect(s.pane()).toBe(880);
    });

    it("a log that follows its output is not scrolled by the wheel, so its bottom does not hold it", () => {
        const s = setup(atBottom);
        expect(s.wheel(s.box, 120)).toBe(true);
        s.geo.scrollHeight = 600; // output arrived; the log pinned itself to its new bottom
        s.geo.scrollTop = 400;
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1240);
    });

    it("never touches Ctrl+wheel (that's zoom) or a horizontal scroll", () => {
        const s = setup(midway);
        s.toEdge(120);
        expect(s.wheel(s.box, 120, { ctrlKey: true })).toBe(false);
        expect(s.wheel(s.box, 0)).toBe(false);
        s.skid(s.box, 120); // the skid is still whole
        expect(s.pane()).toBe(1000);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
    });

    it("a box that grows from fitting to overflowing still hands the wheel on until the wheel scrolls it", () => {
        const s = setup(fitting);
        s.wheel(s.box, 120); // fits: handed on
        expect(s.pane()).toBe(1120);
        s.geo.scrollHeight = 500; // output grew; the preview pins itself to its bottom
        s.geo.scrollTop = 300;
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1240);
        s.toEdge(-120); // scrolled up to its top by the wheel: now the top holds it
        s.skid(s.box, -120);
        expect(s.pane()).toBe(1240);
    });

    it("a box that shrinks to fit after the wheel scrolled it cancels the absorbed notches itself", () => {
        const s = setup(midway);
        s.toEdge(120);
        s.geo.scrollHeight = 200; // its content shrank: nothing left for `contain` to act on
        s.geo.scrollTop = 0;
        for (let i = 0; i < N; i++) expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1000);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(true);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
    });

    // previewBox() is what every transcript preview box attaches.
    it("previewBox() attaches the hand-off, and removes it with its owner", () => {
        const pane = document.createElement("div");
        pane.className = "agent-document";
        let paneTop = 1000;
        Object.defineProperty(pane, "scrollTop", { configurable: true, get: () => paneTop, set: (v) => (paneTop = v) });
        const box = makeBox({ ...fitting });
        pane.append(box);
        document.body.appendChild(pane);
        const dispose = createRoot((d) => {
            previewBox()(box);
            return d;
        });
        const notch = (target: Element) => {
            const e = new WheelEvent("wheel", { deltaY: 120, bubbles: true, cancelable: true });
            Object.defineProperty(e, "wheelDeltaY", { value: -120 });
            target.dispatchEvent(e);
        };
        expect(box.classList.contains("scroll-handoff-box")).toBe(true);
        notch(box);
        expect(paneTop).toBe(1120); // it fits: handed straight on
        dispose();
        expect(box.classList.contains("scroll-handoff-box")).toBe(false);
        notch(box);
        expect(paneTop).toBe(1120);
        pane.remove();
    });

    it("a coalesced event skids the first notches and forwards the one beyond", () => {
        const s = setup(midway);
        s.toEdge(120);
        expect(s.wheel(s.box, (N + 1) * 100, { wheelDeltaY: -(N + 1) * 120 })).toBe(true);
        expect(s.pane()).toBe(1100);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(true);
    });

    it("a coalesced event can finish a skid that a single notch started", () => {
        const s = setup(midway);
        s.toEdge(120);
        s.wheel(s.box, 120);
        expect(s.wheel(s.box, N * 100, { wheelDeltaY: -N * 120 })).toBe(true);
        expect(s.pane()).toBe(1100);
    });

    it("reversing direction: the box scrolls back first, and the edge it reaches holds the wheel again", () => {
        const s = setup(midway);
        s.toEdge(120);
        s.skid(s.box, 120);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
        expect(s.wheel(s.box, -120)).toBe(false); // the box has room upward: native
        expect(s.pane()).toBe(1120);
        s.geo.scrollTop = 0; // which brought it to its top
        s.skid(s.box, -120);
        expect(s.pane()).toBe(1120);
        s.wheel(s.box, -120);
        expect(s.pane()).toBe(1000);
    });

    it("leaving the box for the pane and coming back starts over: at its edge, it hands the wheel on", () => {
        const s = setup(midway);
        s.toEdge(120);
        s.skid(s.box, 120);
        s.wheel(s.box, 120);
        expect(s.pane()).toBe(1120);
        s.wheel(s.row, 120); // over the pane: native, resets the skid
        expect(s.wheel(s.box, 120)).toBe(true);
        expect(s.pane()).toBe(1240);
    });

    it("another box under the pointer does not inherit the hold, nor does the first on the way back", () => {
        const s = setup(midway, atBottom);
        s.toEdge(120);
        s.skid(s.boxes[0], 120);
        s.wheel(s.boxes[0], 120);
        expect(s.pane()).toBe(1120);
        expect(s.wheel(s.boxes[1], 120)).toBe(true);
        expect(s.pane()).toBe(1240);
        expect(s.wheel(s.boxes[0], 120)).toBe(true);
        expect(s.pane()).toBe(1360);
    });

    it("a trackpad gesture that scrolled the box to its edge is absorbed until it pauses", () => {
        const s = setup(midway);
        s.toEdge(30, 0, { notch: false });
        s.wheel(s.box, 30, { notch: false, at: 1000 });
        s.wheel(s.box, 30, { notch: false, at: 1016 });
        s.wheel(s.box, 30, { notch: false, at: 1200 }); // momentum, still under the idle gap
        expect(s.pane()).toBe(1000);
        s.wheel(s.box, 30, { notch: false, at: 1200 + SKID_GESTURE_IDLE_MS }); // a new swipe
        expect(s.pane()).toBe(1030);
    });

    it("shows a line on the edge it is pinned against for the whole skid, then clears", () => {
        vi.useFakeTimers();
        const s = setup(midway);
        expect(s.box.classList.contains("scroll-handoff-box")).toBe(true);
        s.toEdge(120);
        expect(s.box.classList.contains("scroll-handoff-skid--bottom")).toBe(false);
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
        for (let i = 0; i <= SKID_NOTCHES_DEFAULT; i++) expect(s.wheel(s.box, 120)).toBe(false);
        expect(s.pane()).toBe(1000);
        expect(s.box.classList.contains("scroll-handoff-box")).toBe(false);
    });
});

/** The hover panel's case: a box rendered at the document body, handing its wheel to the transcript it sits over. */
describe("attachScrollHandoff for a box outside the transcript (the hover panel)", () => {
    const fits: Geo = { scrollTop: 0, clientHeight: 40, scrollHeight: 40 };

    function setupPanel(given: Geo) {
        const pane = document.createElement("div");
        pane.className = "agent-document";
        let paneTop = 1000;
        Object.defineProperty(pane, "scrollTop", { configurable: true, get: () => paneTop, set: (v) => (paneTop = v) });
        document.body.appendChild(pane);
        const geo = { ...given };
        const panel = makeBox(geo);
        document.body.appendChild(panel);
        const detach = attachScrollHandoff(panel, { pane: () => pane });
        let clock = 0;
        const wheel = (deltaY: number, o: { notch?: boolean } = {}): boolean => {
            const e = new WheelEvent("wheel", { deltaY, bubbles: true, cancelable: true });
            if (o.notch !== false) Object.defineProperty(e, "wheelDeltaY", { value: -Math.sign(deltaY) * 120 });
            clock += 16;
            Object.defineProperty(e, "timeStamp", { value: clock });
            panel.dispatchEvent(e);
            return e.defaultPrevented;
        };
        /** A notch the panel takes itself, which brings it to `top`. */
        const scrollTo = (deltaY: number, top: number) => {
            expect(wheel(deltaY)).toBe(false);
            geo.scrollTop = top;
        };
        return { pane: () => paneTop, panel, geo, wheel, scrollTo, detach };
    }

    it("a one-line panel hands every notch to the transcript", () => {
        const s = setupPanel(fits);
        expect(s.wheel(120)).toBe(true);
        expect(s.pane()).toBe(1120);
        expect(s.wheel(-120)).toBe(true);
        expect(s.pane()).toBe(1000);
    });

    it("a one-line panel hands on a trackpad's continuous input too", () => {
        const s = setupPanel(fits);
        expect(s.wheel(30, { notch: false })).toBe(true);
        expect(s.wheel(18, { notch: false })).toBe(true);
        expect(s.pane()).toBe(1048);
    });

    it("a tall panel scrolls itself while it has room", () => {
        const s = setupPanel(midway);
        expect(s.wheel(120)).toBe(false);
        expect(s.wheel(-120)).toBe(false);
        expect(s.pane()).toBe(1000);
    });

    it("a tall panel the wheel scrolled to its bottom skids, then hands the wheel on, as a preview box does", () => {
        const s = setupPanel(midway);
        s.scrollTo(120, 300);
        expect(s.wheel(120)).toBe(false);
        expect(s.wheel(120)).toBe(false);
        expect(s.pane()).toBe(1000);
        expect(s.wheel(120)).toBe(true);
        expect(s.pane()).toBe(1120);
    });

    it("a tall panel the wheel scrolled to its top skids upward, then hands the wheel on", () => {
        const s = setupPanel(midway);
        s.scrollTo(-120, 0);
        expect(s.wheel(-120)).toBe(false);
        expect(s.wheel(-120)).toBe(false);
        expect(s.wheel(-120)).toBe(true);
        expect(s.pane()).toBe(880);
    });

    it("a tall panel that opens at its edge hands the wheel on at once", () => {
        const s = setupPanel(atTop);
        expect(s.wheel(-120)).toBe(true);
        expect(s.pane()).toBe(880);
    });

    it("tells the transcript about the gesture it relays, so a following transcript sees a wheel-up", () => {
        const s = setupPanel(fits);
        const seen: number[] = [];
        document.querySelector(".agent-document")!.addEventListener("wheel", (e) => seen.push((e as WheelEvent).deltaY), { capture: true });
        s.wheel(-120);
        s.wheel(120);
        expect(seen).toEqual([-120, 120]);
    });

    it("says nothing to the transcript for a notch the panel absorbs or scrolls itself", () => {
        const s = setupPanel(midway);
        const seen: number[] = [];
        document.querySelector(".agent-document")!.addEventListener("wheel", (e) => seen.push((e as WheelEvent).deltaY), { capture: true });
        s.scrollTo(120, 300);
        s.wheel(120);
        s.wheel(120);
        expect(seen).toEqual([]);
        s.wheel(120);
        expect(seen).toEqual([120]);
    });

    it("keeps forwarding once the skid is spent (the relayed wheel does not reset it)", () => {
        const s = setupPanel(midway);
        s.scrollTo(120, 300);
        for (let i = 0; i < 6; i++) s.wheel(120);
        expect(s.pane()).toBe(1000 + 4 * 120);
    });

    it("leaves a wheel alone when the panel has no transcript to hand it to", () => {
        const panel = makeBox(fits);
        document.body.appendChild(panel);
        attachScrollHandoff(panel, { pane: () => null });
        const e = new WheelEvent("wheel", { deltaY: 120, bubbles: true, cancelable: true });
        panel.dispatchEvent(e);
        expect(e.defaultPrevented).toBe(false);
    });

    it("never touches Ctrl+wheel (zoom)", () => {
        const s = setupPanel(fits);
        const e = new WheelEvent("wheel", { deltaY: 120, ctrlKey: true, bubbles: true, cancelable: true });
        s.panel.dispatchEvent(e);
        expect(e.defaultPrevented).toBe(false);
        expect(s.pane()).toBe(1000);
    });
});

describe("nextSkid", () => {
    const box = {};
    const other = {};
    /** By default: at the bottom edge (scrolled to 300), one notch down. */
    const input = (o: Partial<SkidInput> = {}): SkidInput => ({
        box,
        dir: 1,
        deltaY: 100,
        scrollTop: 300,
        atEdge: true,
        limit: N,
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
    /** A notch the box takes itself, from 200; the default input then finds it at 300, its edge. */
    const scrollToEdge = (o: Partial<SkidInput> = {}) => expect(step({ atEdge: false, scrollTop: 200, ...o })).toEqual({ kind: "native" });
    beforeEach(() => {
        s = SKID_IDLE;
    });

    it("an edge the wheel scrolled the box to: absorb the limit's notches, then forward", () => {
        scrollToEdge();
        for (let i = 0; i < N; i++) expect(step()).toEqual({ kind: "absorb" });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
    });

    it("an edge the wheel found the box at: forward at once, and keep forwarding", () => {
        for (let i = 0; i <= N; i++) expect(step()).toEqual({ kind: "forward", deltaY: 100 });
        expect(step({ dir: -1, deltaY: -100, scrollTop: 0 })).toEqual({ kind: "forward", deltaY: -100 });
        expect(s.scrolled).toBe(false);
    });

    it("a notch left to the box counts only if the box moved", () => {
        expect(step({ atEdge: false, scrollTop: 300 })).toEqual({ kind: "native" });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 }); // still at 300: the browser gave it to the pane
        expect(s.scrolled).toBe(false);
        expect(step({ atEdge: false, scrollTop: 300 })).toEqual({ kind: "native" });
        expect(step({ scrollTop: 300.4 })).toEqual({ kind: "forward", deltaY: 100 }); // under a pixel is no move
    });

    it("a box that moved with no notch left to it was not scrolled by the wheel (a log following its output)", () => {
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
        expect(step({ scrollTop: 400 })).toEqual({ kind: "forward", deltaY: 100 });
        expect(s.scrolled).toBe(false);
    });

    it("off the edge is native and starts the skid over", () => {
        scrollToEdge();
        expect(step()).toEqual({ kind: "absorb" });
        expect(step({ atEdge: false, scrollTop: 250 })).toEqual({ kind: "native" });
        for (let i = 0; i < N; i++) expect(step()).toEqual({ kind: "absorb" });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
    });

    it("another box starts with nothing scrolled, and so does the first on the way back", () => {
        scrollToEdge();
        for (let i = 0; i <= N; i++) step();
        expect(step({ box: other })).toEqual({ kind: "forward", deltaY: 100 });
        expect(step()).toEqual({ kind: "forward", deltaY: 100 });
        expect(s.scrolled).toBe(false);
    });

    it("turning the wheel round in a box it scrolled skids again at the other edge", () => {
        scrollToEdge();
        for (let i = 0; i <= N; i++) step();
        expect(s.phase).toBe("spent");
        expect(step({ dir: -1, deltaY: -100, scrollTop: 0 })).toEqual({ kind: "absorb" });
        expect(s.absorbed).toBe(1);
    });

    it("forwards the notches beyond the skid, and says it absorbed some", () => {
        scrollToEdge();
        expect(step({ notches: N + 1, deltaY: (N + 1) * 100 })).toEqual({ kind: "forward", deltaY: 100, absorbed: true });
        expect(s.phase).toBe("spent");
        expect(step({ notches: 2, deltaY: 200 })).toEqual({ kind: "forward", deltaY: 200 });
    });

    it("continuous input: absorbed under the idle gap, forwarded after it", () => {
        scrollToEdge({ notches: null });
        expect(step({ notches: null, timeStamp: 0 })).toEqual({ kind: "absorb" });
        expect(step({ notches: null, timeStamp: SKID_GESTURE_IDLE_MS - 1 })).toEqual({ kind: "absorb" });
        expect(step({ notches: null, timeStamp: 2 * SKID_GESTURE_IDLE_MS })).toEqual({ kind: "forward", deltaY: 100 });
    });

    it("a limit of 0 forwards at once, and a bigger one absorbs that many", () => {
        scrollToEdge({ limit: 0 });
        expect(step({ limit: 0 })).toEqual({ kind: "forward", deltaY: 100 });
        s = SKID_IDLE;
        expect(step({ limit: 0, atEdge: false })).toEqual({ kind: "native" });
        s = SKID_IDLE;
        scrollToEdge({ limit: 5 });
        for (let i = 0; i < 5; i++) expect(step({ limit: 5 })).toEqual({ kind: "absorb" });
        expect(step({ limit: 5 })).toEqual({ kind: "forward", deltaY: 100 });
    });
});
