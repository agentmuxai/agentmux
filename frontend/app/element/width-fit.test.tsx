// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fitToWidth, useWidthFit } from "./width-fit";

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

/** An element whose content needs `need` px at the tier it has, and whose width is `width`. */
function sized(width: number, needAt: (tier: string | null) => number): HTMLElement {
    const el = document.createElement("div");
    Object.defineProperty(el, "clientWidth", { get: () => width });
    Object.defineProperty(el, "scrollWidth", { get: () => Math.max(width, needAt(el.getAttribute("data-fit"))) });
    return el;
}

const NEED: Record<string, number> = { none: 300, trim: 200, min: 120 };
const need = (tier: string | null) => NEED[tier ?? "none"];

describe("fitToWidth", () => {
    it("leaves everything shown when it fits", () => {
        const el = sized(320, need);
        fitToWidth(el, ["trim", "min"]);
        expect(el.hasAttribute("data-fit")).toBe(false);
    });

    it("takes the first tier that fits, not the last", () => {
        const el = sized(250, need);
        fitToWidth(el, ["trim", "min"]);
        expect(el.getAttribute("data-fit")).toBe("trim");
    });

    it("goes on to the last tier, and stays there, when nothing fits", () => {
        const el = sized(60, need);
        fitToWidth(el, ["trim", "min"]);
        expect(el.getAttribute("data-fit")).toBe("min");
    });

    it("starts again from the full set, so a wider bar gets its parts back", () => {
        let width = 130;
        const el = document.createElement("div");
        Object.defineProperty(el, "clientWidth", { get: () => width });
        Object.defineProperty(el, "scrollWidth", { get: () => Math.max(width, need(el.getAttribute("data-fit"))) });
        fitToWidth(el, ["trim", "min"]);
        expect(el.getAttribute("data-fit")).toBe("min");
        width = 400;
        fitToWidth(el, ["trim", "min"]);
        expect(el.hasAttribute("data-fit")).toBe(false);
    });

    it("sees a row pushed to the right edge overrun on the left, which scrollWidth does not", () => {
        const el = sized(250, () => 0);
        el.getBoundingClientRect = () => ({ left: 100, width: 250 }) as DOMRect;
        const child = document.createElement("span");
        child.getBoundingClientRect = () =>
            ({ left: el.getAttribute("data-fit") === "min" ? 100 : 80, width: 24 }) as DOMRect;
        el.append(child);
        fitToWidth(el, ["trim", "min"]);
        expect(el.getAttribute("data-fit")).toBe("min");
    });

    it("gives a child with no box (display: contents) no say", () => {
        const el = sized(250, () => 0);
        el.getBoundingClientRect = () => ({ left: 100, width: 250 }) as DOMRect;
        const child = document.createElement("span");
        child.getBoundingClientRect = () => ({ left: 0, width: 0 }) as DOMRect;
        el.append(child);
        fitToWidth(el, ["trim", "min"]);
        expect(el.hasAttribute("data-fit")).toBe(false);
    });

    it("ignores a pixel of rounding at a fractional zoom", () => {
        const el = sized(299, need);
        fitToWidth(el, ["trim", "min"]);
        expect(el.hasAttribute("data-fit")).toBe(false);
    });
});

describe("useWidthFit", () => {
    class FakeObserver {
        static last: FakeObserver | undefined;
        targets: Element[] = [];
        disconnected = false;
        constructor(public callback: () => void) {
            FakeObserver.last = this;
        }
        observe(el: Element) {
            this.targets.push(el);
        }
        disconnect() {
            this.disconnected = true;
        }
    }

    it("fits when its element or a watched part changes size, and stops on cleanup", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        const el = sized(250, need);
        const part = document.createElement("span");
        const view = render(() => {
            useWidthFit(() => el, ["trim", "min"], () => [part]);
            return null;
        });
        const observer = FakeObserver.last!;
        expect(observer.targets).toEqual([el, part]);
        expect(el.hasAttribute("data-fit")).toBe(false);
        observer.callback();
        expect(el.getAttribute("data-fit")).toBe("trim");
        view.unmount();
        expect(observer.disconnected).toBe(true);
    });

    it("does nothing where there is no ResizeObserver", () => {
        vi.stubGlobal("ResizeObserver", undefined);
        const el = sized(250, need);
        render(() => {
            useWidthFit(() => el, ["trim", "min"]);
            return null;
        });
        expect(el.hasAttribute("data-fit")).toBe(false);
    });

    it("fits again, a frame later, when the trigger changes", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
            cb();
            return 1;
        });
        vi.stubGlobal("cancelAnimationFrame", () => {});
        let width = 320;
        const el = document.createElement("div");
        Object.defineProperty(el, "clientWidth", { get: () => width });
        Object.defineProperty(el, "scrollWidth", { get: () => Math.max(width, need(el.getAttribute("data-fit"))) });
        const [zoom, setZoom] = createSignal(1);
        render(() => {
            useWidthFit(() => el, ["trim", "min"], () => [], zoom);
            return null;
        });
        expect(el.hasAttribute("data-fit")).toBe(false);
        width = 250;
        setZoom(1.5);
        expect(el.getAttribute("data-fit")).toBe("trim");
    });
});
