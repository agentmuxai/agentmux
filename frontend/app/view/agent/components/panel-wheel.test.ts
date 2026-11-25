// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
import { attachScrollHandoff } from "./scroll-handoff";

interface Geo {
    top: number;
    left: number;
    room: number;
    roomX: number;
}

/** A part with stubbed geometry: `room` pixels to scroll down, `roomX` sideways. 0 means it fits. */
function part(overflow: string, g: Partial<Geo> = {}): { el: HTMLElement; g: Geo } {
    const geo: Geo = { top: 0, left: 0, room: 0, roomX: 0, ...g };
    const el = document.createElement("div");
    el.style.setProperty("overflow-y", overflow);
    el.style.setProperty("overflow-x", overflow);
    const define = (name: string, get: () => number, set?: (v: number) => void) =>
        Object.defineProperty(el, name, { configurable: true, get, set });
    define("clientHeight", () => 200);
    define("scrollHeight", () => 200 + geo.room);
    define("clientWidth", () => 300);
    define("scrollWidth", () => 300 + geo.roomX);
    define("scrollTop", () => geo.top, (v) => (geo.top = Math.max(0, Math.min(geo.room, v))));
    define("scrollLeft", () => geo.left, (v) => (geo.left = Math.max(0, Math.min(geo.roomX, v))));
    return { el, g: geo };
}

/** The transcript with a row, and the hover panel at the document body (outside it), as `PeekOverlay` renders it. */
function setup(panelGeo: Partial<Geo> = {}, overflow = "auto") {
    const pane = document.createElement("div");
    pane.className = "agent-document";
    let paneTop = 1000;
    Object.defineProperty(pane, "scrollTop", { configurable: true, get: () => paneTop, set: (v) => (paneTop = v) });
    const row = document.createElement("div");
    pane.appendChild(row);
    document.body.appendChild(pane);
    const panel = part(overflow, panelGeo);
    document.body.appendChild(panel.el);
    const detach = attachScrollHandoff(panel.el, { pane: () => row.closest<HTMLElement>(".agent-document"), keepWheel: true });
    /** One wheel event; true when the page's own scrolling was cancelled. */
    const wheel = (target: Element, init: WheelEventInit = { deltaY: 120 }) => {
        const e = new WheelEvent("wheel", { bubbles: true, cancelable: true, ...init });
        target.dispatchEvent(e);
        return e.defaultPrevented;
    };
    return { pane: () => paneTop, panel, wheel, detach };
}

/** A log inside the panel, the part that scrolls when the panel itself fits. */
function withLog(s: ReturnType<typeof setup>, g: Partial<Geo>, overflow = "auto") {
    const log = part(overflow, g);
    s.panel.el.appendChild(log.el);
    const text = document.createElement("span");
    log.el.appendChild(text);
    const header = document.createElement("div");
    s.panel.el.insertBefore(header, log.el);
    return { log, text, header };
}

afterEach(() => {
    document.body.innerHTML = "";
});

describe("the wheel over the hover panel keeps to the panel when it can scroll", () => {
    it("a panel that scrolls, mid-way: the browser scrolls it, the transcript stays", () => {
        const s = setup({ room: 400, top: 100 });
        expect(s.wheel(s.panel.el, { deltaY: 120 })).toBe(false);
        expect(s.wheel(s.panel.el, { deltaY: -120 })).toBe(false);
        expect(s.pane()).toBe(1000);
    });

    it("at the bottom end the wheel does nothing: no skid, no hand-off, every notch after it", () => {
        const s = setup({ room: 400, top: 400 });
        for (let i = 0; i < 6; i++) expect(s.wheel(s.panel.el, { deltaY: 120 })).toBe(true);
        expect(s.pane()).toBe(1000);
    });

    it("at the top end the wheel does nothing", () => {
        const s = setup({ room: 400, top: 0 });
        for (let i = 0; i < 6; i++) expect(s.wheel(s.panel.el, { deltaY: -120 })).toBe(true);
        expect(s.pane()).toBe(1000);
    });

    it("a trackpad's momentum past the end is held event by event", () => {
        const s = setup({ room: 400, top: 399.5 });
        const results = [40, 30, 22, 15, 9, 4, 2, 1].map((deltaY) => s.wheel(s.panel.el, { deltaY }));
        expect(results.slice(1).every(Boolean)).toBe(true);
        expect(s.pane()).toBe(1000);
    });

    it("a panel with nothing to scroll still hands every notch to the transcript", () => {
        const s = setup({ room: 0 });
        expect(s.wheel(s.panel.el, { deltaY: 120 })).toBe(true);
        expect(s.pane()).toBe(1120);
        expect(s.wheel(s.panel.el, { deltaY: -120 })).toBe(true);
        expect(s.pane()).toBe(1000);
    });

    it("a part inside a panel that fits scrolls natively mid-way, and holds at its ends", () => {
        const s = setup({ room: 0 });
        const { log, text } = withLog(s, { room: 300, top: 100 });
        expect(s.wheel(text, { deltaY: 120 })).toBe(false);
        log.g.top = 300;
        expect(s.wheel(text, { deltaY: 120 })).toBe(true);
        log.g.top = 0;
        expect(s.wheel(text, { deltaY: -120 })).toBe(true);
        expect(s.pane()).toBe(1000);
    });

    it("a nested block at its end hands the wheel to the part around it, inside the panel, never past it", () => {
        const s = setup({ room: 500, top: 50 });
        const { log, text } = withLog(s, { room: 300, top: 300 });
        expect(s.wheel(text, { deltaY: 120 })).toBe(true);
        expect(s.panel.g.top).toBe(170);
        expect(log.g.top).toBe(300);
        s.panel.g.top = 500;
        expect(s.wheel(text, { deltaY: 120 })).toBe(true);
        expect(s.panel.g.top).toBe(500);
        expect(s.pane()).toBe(1000);
    });

    it("the wheel over the panel's header scrolls the part that can, and the transcript stays", () => {
        const s = setup({ room: 0 });
        const { log, header } = withLog(s, { room: 300, top: 0 });
        expect(s.wheel(header, { deltaY: 120 })).toBe(true);
        expect(log.g.top).toBe(120);
        expect(s.pane()).toBe(1000);
        log.g.top = 300;
        expect(s.wheel(header, { deltaY: 120 })).toBe(true);
        expect(log.g.top).toBe(300);
        expect(s.pane()).toBe(1000);
    });

    it("a wheel in lines moves a part by hand by lines, not pixels", () => {
        const s = setup({ room: 0 });
        const { log, header } = withLog(s, { room: 600, top: 0 });
        s.wheel(header, { deltaY: 3, deltaMode: 1 });
        expect(log.g.top).toBe(120);
    });

    it("a sideways wheel over a wide command scrolls it, and holds at its ends", () => {
        const s = setup({ room: 0, roomX: 0 });
        const { log, text } = withLog(s, { room: 0, roomX: 500, left: 100 });
        expect(s.wheel(text, { deltaX: 120, deltaY: 0 })).toBe(false);
        log.g.left = 500;
        expect(s.wheel(text, { deltaX: 120, deltaY: 0 })).toBe(true);
        log.g.left = 0;
        expect(s.wheel(text, { deltaX: -120, deltaY: 0 })).toBe(true);
        expect(s.pane()).toBe(1000);
    });

    it("shift+wheel over a wide command goes sideways the same way", () => {
        const s = setup({ room: 0, roomX: 0 });
        const { log, text } = withLog(s, { room: 0, roomX: 500, left: 500 });
        expect(s.wheel(text, { deltaY: 120, shiftKey: true })).toBe(true);
        expect(s.pane()).toBe(1000);
        log.g.left = 100;
        expect(s.wheel(text, { deltaY: 120, shiftKey: true })).toBe(false);
    });

    it("a sideways wheel over a panel with no sideways scroll does nothing to the transcript", () => {
        const s = setup({ room: 0 });
        expect(s.wheel(s.panel.el, { deltaX: 120, deltaY: 0 })).toBe(false);
        expect(s.pane()).toBe(1000);
    });

    it("ctrl+wheel is the pane's zoom: left alone", () => {
        const s = setup({ room: 400, top: 400 });
        expect(s.wheel(s.panel.el, { deltaY: 120, ctrlKey: true })).toBe(false);
        expect(s.pane()).toBe(1000);
    });

    it("a box that does not ask for it still skids and hands on at its edge, as before", () => {
        const s = setup({ room: 400, top: 400 });
        s.detach();
        const detach = attachScrollHandoff(s.panel.el, { pane: () => document.querySelector<HTMLElement>(".agent-document") });
        expect(s.wheel(s.panel.el, { deltaY: 120 })).toBe(true);
        expect(s.pane()).toBe(1120);
        detach();
    });
});
