// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fitToWidth, useWidthFit } from "./width-fit";

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.useRealTimers();
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
    /** The window's one observer, as the tests drive it: `resized(...)` is a frame in which those targets changed size. */
    class FakeObserver {
        static made: FakeObserver[] = [];
        targets: Element[] = [];
        disconnected = false;
        constructor(public callback: (entries: { target: Element }[]) => void) {
            FakeObserver.made.push(this);
        }
        observe(el: Element) {
            this.targets.push(el);
        }
        unobserve(el: Element) {
            this.targets = this.targets.filter((t) => t !== el);
        }
        disconnect() {
            this.disconnected = true;
        }
        resized(...targets: Element[]) {
            this.callback(targets.map((target) => ({ target })));
        }
    }
    const theObserver = () => FakeObserver.made[FakeObserver.made.length - 1];

    /** An element like `sized`, whose width and needs can change, and which records what is done to it. */
    function tracked(
        name: string,
        log: string[],
        box: { width: number; need: (tier: string | null) => number },
    ): HTMLElement {
        const el = document.createElement("div");
        Object.defineProperty(el, "clientWidth", { get: () => box.width });
        Object.defineProperty(el, "scrollWidth", {
            get: () => {
                log.push(`measure ${name}`);
                return Math.max(box.width, box.need(el.getAttribute("data-fit")));
            },
        });
        const set = el.setAttribute.bind(el);
        el.setAttribute = (key: string, value: string) => {
            log.push(`set ${name} ${value}`);
            set(key, value);
        };
        const remove = el.removeAttribute.bind(el);
        el.removeAttribute = (key: string) => {
            log.push(`clear ${name}`);
            remove(key);
        };
        document.body.append(el);
        return el;
    }

    it("fits when its element or a watched part changes size, and stops on cleanup", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        const el = sized(250, need);
        const part = document.createElement("span");
        const view = render(() => {
            useWidthFit(() => el, ["trim", "min"], () => [part]);
            return null;
        });
        const observer = theObserver();
        expect(observer.targets).toEqual([el, part]);
        expect(el.hasAttribute("data-fit")).toBe(false);
        observer.resized(el);
        expect(el.getAttribute("data-fit")).toBe("trim");
        view.unmount();
        expect(observer.targets).toEqual([]);
        expect(observer.disconnected).toBe(true);
    });

    it("watches every fitted element of the window with one observer", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        const before = FakeObserver.made.length;
        const a = sized(250, need);
        const b = sized(250, need);
        render(() => {
            useWidthFit(() => a, ["trim", "min"]);
            useWidthFit(() => b, ["trim", "min"]);
            return null;
        });
        expect(FakeObserver.made.length).toBe(before + 1);
        expect(theObserver().targets).toEqual([a, b]);
    });

    it("writes every element's tier before it measures any, a step at a time", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        const log: string[] = [];
        const tiers = ["trim", "min"];
        const a = tracked("a", log, { width: 250, need });
        const b = tracked("b", log, { width: 150, need });
        render(() => {
            useWidthFit(() => a, tiers);
            useWidthFit(() => b, tiers);
            return null;
        });
        theObserver().resized(a, b);
        expect(a.getAttribute("data-fit")).toBe("trim");
        expect(b.getAttribute("data-fit")).toBe("min");
        expect(log.filter((line, i) => line !== log[i - 1])).toEqual([
            "measure a",
            "measure b",
            "set a trim",
            "set b trim",
            "measure a",
            "measure b",
            "set b min",
            "measure b",
        ]);
    });

    it("only measures an element that is still narrower than its tier below needed", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        vi.useFakeTimers();
        const log: string[] = [];
        const box = { width: 250, need };
        const el = tracked("el", log, box);
        render(() => {
            useWidthFit(() => el, ["trim", "min"]);
            return null;
        });
        theObserver().resized(el);
        expect(el.getAttribute("data-fit")).toBe("trim");
        log.length = 0;
        box.width = 280;
        theObserver().resized(el);
        expect(el.getAttribute("data-fit")).toBe("trim");
        expect(log.filter((line) => !line.startsWith("measure"))).toEqual([]);
    });

    it("gives the parts back when the element is as wide as its tier below needed", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        const box = { width: 250, need };
        const el = tracked("el", [], box);
        render(() => {
            useWidthFit(() => el, ["trim", "min"]);
            return null;
        });
        theObserver().resized(el);
        expect(el.getAttribute("data-fit")).toBe("trim");
        box.width = 300;
        theObserver().resized(el);
        expect(el.hasAttribute("data-fit")).toBe(false);
    });

    it("hides more when an element that was only measured no longer fits its own tier", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        vi.useFakeTimers();
        const box = { width: 250, need };
        const el = tracked("el", [], box);
        render(() => {
            useWidthFit(() => el, ["trim", "min"]);
            return null;
        });
        theObserver().resized(el);
        box.width = 150;
        theObserver().resized(el);
        expect(el.getAttribute("data-fit")).toBe("min");
    });

    it("fits from the full set once resizing has stopped, so content that shrank meanwhile gets its parts back", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        vi.useFakeTimers();
        let full = 300;
        const box = { width: 250, need: (tier: string | null) => (tier === null ? full : need(tier)) };
        const el = tracked("el", [], box);
        render(() => {
            useWidthFit(() => el, ["trim", "min"]);
            return null;
        });
        theObserver().resized(el);
        expect(el.getAttribute("data-fit")).toBe("trim");
        full = 240;
        box.width = 260;
        theObserver().resized(el);
        expect(el.getAttribute("data-fit")).toBe("trim");
        vi.advanceTimersByTime(250);
        expect(el.hasAttribute("data-fit")).toBe(false);
    });

    it("fits from the full set when a watched part changes size", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        vi.useFakeTimers();
        let full = 300;
        const box = { width: 250, need: (tier: string | null) => (tier === null ? full : need(tier)) };
        const el = tracked("el", [], box);
        const part = document.createElement("span");
        render(() => {
            useWidthFit(() => el, ["trim", "min"], () => [part]);
            return null;
        });
        theObserver().resized(el);
        expect(el.getAttribute("data-fit")).toBe("trim");
        full = 240;
        theObserver().resized(part);
        expect(el.hasAttribute("data-fit")).toBe(false);
    });

    it("leaves an element that was unmounted while resizing alone", () => {
        vi.stubGlobal("ResizeObserver", FakeObserver);
        vi.useFakeTimers();
        const log: string[] = [];
        const box = { width: 250, need };
        const el = tracked("el", log, box);
        const view = render(() => {
            useWidthFit(() => el, ["trim", "min"]);
            return null;
        });
        theObserver().resized(el);
        box.width = 280;
        theObserver().resized(el);
        view.unmount();
        log.length = 0;
        vi.advanceTimersByTime(250);
        expect(log).toEqual([]);
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
