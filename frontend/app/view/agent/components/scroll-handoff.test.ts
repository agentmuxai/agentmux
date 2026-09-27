// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { attachScrollHandoff } from "./scroll-handoff";

/** A capped box inside the pane's `.agent-document`, with stubbed geometry. */
function setup(g: { scrollTop: number; clientHeight: number; scrollHeight: number }) {
    const pane = document.createElement("div");
    pane.className = "agent-document";
    let paneTop = 1000;
    Object.defineProperty(pane, "scrollTop", { configurable: true, get: () => paneTop, set: (v) => (paneTop = v) });
    const box = document.createElement("div");
    Object.defineProperty(box, "scrollTop", { configurable: true, get: () => g.scrollTop });
    Object.defineProperty(box, "clientHeight", { configurable: true, get: () => g.clientHeight });
    Object.defineProperty(box, "scrollHeight", { configurable: true, get: () => g.scrollHeight });
    pane.appendChild(box);
    document.body.appendChild(pane);
    const detach = attachScrollHandoff(box);
    const wheel = (deltaY: number, ctrlKey = false) => {
        const e = new WheelEvent("wheel", { deltaY, ctrlKey, bubbles: true, cancelable: true });
        box.dispatchEvent(e);
        return e.defaultPrevented;
    };
    return { pane: () => paneTop, wheel, detach };
}

afterEach(() => {
    document.body.innerHTML = "";
});

// SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §2: once a capped box can't
// scroll further in the wheel's direction, the next tick moves the pane.
describe("attachScrollHandoff", () => {
    it("hands a wheel down at the bottom to the pane", () => {
        const s = setup({ scrollTop: 300, clientHeight: 200, scrollHeight: 500 });
        expect(s.wheel(120)).toBe(true);
        expect(s.pane()).toBe(1120);
    });

    it("hands a wheel up at the top to the pane", () => {
        const s = setup({ scrollTop: 0, clientHeight: 200, scrollHeight: 500 });
        expect(s.wheel(-120)).toBe(true);
        expect(s.pane()).toBe(880);
    });

    it("lets the box scroll itself while it has room", () => {
        const s = setup({ scrollTop: 100, clientHeight: 200, scrollHeight: 500 });
        expect(s.wheel(120)).toBe(false);
        expect(s.wheel(-120)).toBe(false);
        expect(s.pane()).toBe(1000);
    });

    it("never forwards Ctrl+wheel (that's zoom)", () => {
        const s = setup({ scrollTop: 300, clientHeight: 200, scrollHeight: 500 });
        expect(s.wheel(120, true)).toBe(false);
        expect(s.pane()).toBe(1000);
    });

    it("forwards from a box whose content doesn't overflow", () => {
        const s = setup({ scrollTop: 0, clientHeight: 200, scrollHeight: 200 });
        expect(s.wheel(120)).toBe(true);
        expect(s.pane()).toBe(1120);
    });

    it("stops forwarding once detached", () => {
        const s = setup({ scrollTop: 300, clientHeight: 200, scrollHeight: 500 });
        s.detach();
        expect(s.wheel(120)).toBe(false);
        expect(s.pane()).toBe(1000);
    });
});
