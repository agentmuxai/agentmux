// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane header's scroll range keeps the active tab in view (#46761): the tab strip inside the
 * range does not scroll itself, so its reveal scrolls the range. A selection, a narrowed pane, the
 * pane taking the focus and a click or a key in the pane bring the active tab back, clear of the
 * range's 20 px fade; the person's own scroll of the range, a press on one of its controls and a
 * control in it that holds the keyboard focus are left alone.
 *
 * jsdom has no layout, so the range's geometry is a small model (tabs of a fixed width, then the
 * "+" and a control) behind stubbed `scrollWidth`, `scrollLeft` and `getBoundingClientRect`, and
 * animation frames run when the test says so.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, onCleanup, onMount } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/motion/motion", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/motion/motion")>()),
    // Every reveal is an instant write: the tween has its own tests (strip-reveal.test.ts).
    shouldReduceMotion: () => true,
}));

import { installPaneInteractionReveal, revealPaneStrips } from "@/app/element/pane-strip-registry";
import { PaneTabStrip } from "@/app/element/PaneTabStrip";
import { useHeaderScrollRange } from "./header-scroll-range";

const TABS = 10;
const TAB_W = 100;
const ADD_W = 40;
const AUX_W = 60;
const CLIENT_W = 300;
const FADE = 20;
const SCROLL_W = TABS * TAB_W + ADD_W + AUX_W;
const MAX = SCROLL_W - CLIENT_W;

let frames = new Map<number, FrameRequestCallback>();
let nextFrame = 1;
function flush(): void {
    for (let i = 0; i < 12 && frames.size; i++) {
        const batch = [...frames.values()];
        frames = new Map();
        for (const cb of batch) cb(0);
    }
}

let resizeCallbacks: (() => void)[] = [];
class FakeResizeObserver {
    private cb: () => void;
    constructor(callback: ResizeObserverCallback) {
        this.cb = () => callback([], this as unknown as ResizeObserver);
        resizeCallbacks.push(this.cb);
    }
    observe() {}
    unobserve() {}
    disconnect() {
        resizeCallbacks = resizeCallbacks.filter((cb) => cb !== this.cb);
    }
}

/** The range's stubbed layout: where it is scrolled to, and every write to `scrollLeft`. */
const range = { el: undefined as HTMLElement | undefined, value: 0, writes: [] as number[] };
let rectSpy: ReturnType<typeof vi.spyOn>;
let now: ReturnType<typeof vi.spyOn>;

function stubRange(el: HTMLElement): void {
    range.el = el;
    const fixed = (n: number) => ({ get: () => n, configurable: true });
    Object.defineProperty(el, "scrollWidth", fixed(SCROLL_W));
    Object.defineProperty(el, "clientWidth", fixed(CLIENT_W));
    Object.defineProperty(el, "offsetWidth", fixed(CLIENT_W));
    Object.defineProperty(el, "scrollLeft", {
        get: () => range.value,
        set: (v: number) => {
            range.writes.push(v);
            range.value = Math.min(MAX, Math.max(0, v));
            el.dispatchEvent(new Event("scroll"));
        },
        configurable: true,
    });
}

beforeEach(() => {
    frames = new Map();
    nextFrame = 1;
    resizeCallbacks = [];
    range.el = undefined;
    range.value = 0;
    range.writes = [];
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        frames.set(nextFrame, cb);
        return nextFrame++;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    now = vi.spyOn(performance, "now").mockReturnValue(10_000);
    rectSpy = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
        const rect = (left: number, width: number) => ({ left, right: left + width, width, top: 0, bottom: 20, height: 20 }) as DOMRect;
        if (this === range.el) return rect(0, CLIENT_W);
        if (this.dataset.tabId) return rect(Number(this.dataset.tabId.slice(1)) * TAB_W - range.value, TAB_W);
        return rect(0, 0);
    });
});

afterEach(() => {
    cleanup();
    rectSpy.mockRestore();
    now.mockRestore();
    vi.unstubAllGlobals();
});

/** The scroll that brings tab `i` in from the right, to just clear of the right fade. */
const toRight = (i: number) => (i + 1) * TAB_W - CLIENT_W + FADE;
/** Whether tab `i` is whole inside the range and clear of the fades that show. */
function isRevealed(i: number): boolean {
    const left = i * TAB_W - range.value;
    const leftFade = range.value > 1 ? FADE : 0;
    const rightFade = range.value < MAX - 1 ? FADE : 0;
    return left >= leftFade && left + TAB_W <= CLIENT_W - rightFade;
}

function mountHeader(active = "t5") {
    const ids = Array.from({ length: TABS }, (_, i) => `t${i}`);
    const [activeId, setActiveId] = createSignal(active);
    let root!: HTMLDivElement;
    let header!: HTMLDivElement;
    render(() => {
        // Built before the header around it, as PaneHeaderTabStrip builds it.
        const strip = (
            <PaneTabStrip tabs={ids} activeId={activeId()} getId={(id) => id} getLabel={(id) => id} onActivate={() => {}} onAdd={() => {}} />
        );
        const scroll = useHeaderScrollRange(() => header);
        onMount(() => onCleanup(installPaneInteractionReveal(root)));
        return (
            <div data-role="pane" ref={root}>
                <div class="header" ref={header}>
                    <div class="block-frame-header-scroll" ref={scroll.setRange}>
                        {strip}
                        <button class="aux">aux</button>
                    </div>
                    <button class="pinned">minimize</button>
                </div>
                <textarea class="content" />
            </div>
        );
    });
    const el = root.querySelector<HTMLElement>(".block-frame-header-scroll")!;
    stubRange(el);
    // Let the range's scroll edges measure the stubbed layout, then run the mount reveal.
    el.dispatchEvent(new Event("scroll"));
    flush();
    const q = (selector: string) => root.querySelector<HTMLElement>(selector)!;
    return { root, header, el, content: q(".content"), aux: q(".aux"), pinned: q(".pinned"), setActiveId };
}

/** The person wheels the header by `dy` px. */
function wheel(target: HTMLElement, dy: number): void {
    // A trackpad-sized delta moves the range by a tenth of it (strip-wheel.ts).
    fireEvent.wheel(target, { deltaY: dy * 10 });
    flush();
}

describe("the header's scroll range reveals the active tab", () => {
    it("on mount, by the least it takes to bring it clear of the range's fade", () => {
        mountHeader("t5");
        expect(range.writes).toEqual([toRight(5)]);
        expect(isRevealed(5)).toBe(true);
    });

    it("the last tab comes in beside the fade: the range is not scrolled to its end, where the header's controls are", () => {
        mountHeader("t9");
        expect(range.value).toBe(toRight(9));
        expect(range.value).toBeLessThan(MAX);
        expect(isRevealed(9)).toBe(true);
    });

    it("when another tab is selected", () => {
        const h = mountHeader("t0");
        expect(range.value).toBe(0);
        h.setActiveId("t7");
        flush();
        expect(range.value).toBe(toRight(7));
    });

    it("a tab off the left comes back to just clear of the left fade", () => {
        const h = mountHeader("t9");
        h.setActiveId("t3");
        flush();
        expect(range.value).toBe(3 * TAB_W - FADE);
        expect(isRevealed(3)).toBe(true);
    });

    it("a key or a click in the pane's body brings back the tab the person scrolled away from", () => {
        const h = mountHeader("t0");
        wheel(h.header, 500);
        expect(range.value).toBe(500);
        expect(isRevealed(0)).toBe(false);
        fireEvent.keyDown(h.content, { key: "a" });
        flush();
        expect(range.value).toBe(0);
        wheel(h.header, 500);
        fireEvent.pointerDown(h.content);
        flush();
        expect(range.value).toBe(0);
    });

    it("the pane taking the focus brings it back", () => {
        const h = mountHeader("t0");
        wheel(h.pinned, 400);
        expect(range.value).toBe(400);
        revealPaneStrips(h.root, "focus");
        flush();
        expect(range.value).toBe(0);
    });
});

describe("the header's scroll range is left where the person put it", () => {
    it("by a layout change after their wheel, over a tab, a control or the pinned buttons", () => {
        const h = mountHeader("t0");
        for (const target of [h.el.querySelector<HTMLElement>("[data-tab-id]")!, h.aux, h.pinned]) {
            wheel(target, 100);
            const at = range.value;
            expect(at).toBeGreaterThan(0);
            resizeCallbacks.slice().forEach((cb) => cb());
            flush();
            expect(range.value).toBe(at);
        }
    });

    it("a wheel over a range with nothing to scroll is not their scroll", () => {
        const h = mountHeader("t0");
        Object.defineProperty(h.el, "scrollWidth", { get: () => CLIENT_W, configurable: true });
        range.writes = [];
        wheel(h.header, 100);
        expect(range.writes).toEqual([]);
    });

    it("by a press on one of its controls, and by the focus that press gives the pane", () => {
        const h = mountHeader("t0");
        wheel(h.header, 500);
        fireEvent.pointerDown(h.aux);
        flush();
        expect(range.value).toBe(500);
        now.mockReturnValue(10_100);
        revealPaneStrips(h.root, "focus");
        flush();
        expect(range.value).toBe(500);
    });

    it("while one of its controls holds the keyboard focus, whatever is pressed elsewhere in the pane", () => {
        const h = mountHeader("t0");
        wheel(h.header, 500);
        h.aux.focus();
        fireEvent.keyDown(h.pinned, { key: "Tab" });
        flush();
        expect(range.value).toBe(500);
        // A new selection is still shown.
        h.setActiveId("t2");
        flush();
        expect(isRevealed(2)).toBe(true);
    });
});
