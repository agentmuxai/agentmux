// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's strips keep their selected tab in view: the pane tab strip and the
 * document tab strip, each with ~20 tabs, on every trigger of
 * docs/specs/SPEC_PANE_TAB_STRIP_REVEAL_SELECTED_2026_10_15.md §3. A tab already
 * in view does not move; one partly or wholly out of view moves by the least it
 * takes to clear the edge fade (never to the left edge), over a short tween.
 *
 * jsdom has no layout, so each strip's geometry is a small model (tabs of a
 * fixed width in a strip of a fixed width) behind stubbed `scrollWidth`,
 * `scrollLeft` and `getBoundingClientRect`, and animation frames run when the
 * test says so, on a clock the test advances.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { batch, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let reduceTransition = false;
/** The Animations setting's factor: 1 is the designed timing, 0 is off. */
let motionFactor = 1;
vi.mock("@/app/motion/motion", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/motion/motion")>()),
    shouldReduceMotion: (category: string) => reduceTransition && category === "transition",
    scaledMs: (ms: number) => ms * motionFactor,
}));

import { setKeepAliveBlockDormant } from "@/app/store/block-component-registry";
import { WindowTabDisplayedProvider } from "@/app/workspace/window-tab-visibility";
import { installPaneInteractionReveal, revealPaneStrips } from "./pane-strip-registry";
import { PaneTabStrip } from "./PaneTabStrip";

type Kind = "pane" | "doc";

const TAB_W = 100;
const CLIENT_W = 400;
const FADE = 28;
const ADD_W = 40;

interface Geometry {
    tabs: number;
    tabW: number;
    client: number;
    zoom: number;
}

/** One strip's stubbed layout, and what was done to it. */
class StripModel {
    value = 0;
    /** Every write to `scrollLeft`: one for an instant reveal, a run of steps for a tween. */
    writes: number[] = [];
    reads = 0;
    constructor(
        readonly el: HTMLElement,
        readonly geo: Geometry
    ) {
        const model = this;
        const get = (n: () => number) => ({ get: () => (model.reads++, n()), configurable: true });
        Object.defineProperty(el, "scrollWidth", get(() => geo.tabs * geo.tabW + ADD_W));
        Object.defineProperty(el, "clientWidth", get(() => geo.client));
        Object.defineProperty(el, "offsetWidth", get(() => geo.client));
        Object.defineProperty(el, "scrollLeft", {
            get: () => (model.reads++, model.value),
            set: (v: number) => {
                model.writes.push(v);
                model.set(v);
            },
            configurable: true,
        });
        // The browser's own smooth scroll has a length nobody sets: a reveal must not use it.
        el.scrollTo = (() => {
            throw new Error("a reveal used the browser's smooth scroll");
        }) as typeof el.scrollTo;
    }
    get max(): number {
        return this.geo.tabs * this.geo.tabW + ADD_W - this.geo.client;
    }
    set(v: number): void {
        this.value = Math.min(this.max, Math.max(0, v));
        this.el.dispatchEvent(new Event("scroll"));
    }
    tab(id: string): HTMLElement {
        return this.el.querySelector<HTMLElement>(`[data-tab-id="${id}"]`)!;
    }
    /** The tab's rect in viewport px, from the strip's left edge. */
    rect(tab: HTMLElement) {
        const i = [...this.el.querySelectorAll(".pane-tab")].indexOf(tab);
        const left = (i * this.geo.tabW - this.value) * this.geo.zoom;
        return { left, right: left + this.geo.tabW * this.geo.zoom };
    }
    /** Whether the tab is whole, inside the strip and clear of the fades that show (flush left it needs none: the strip drops the left fade there). */
    isRevealed(id: string): boolean {
        const r = this.rect(this.tab(id));
        const z = this.geo.zoom;
        const left = this.value > 1 ? FADE : 0;
        const right = this.value < this.max - 1 ? FADE : 0;
        const flush = Math.abs(r.left) < 0.01;
        return (flush || r.left >= left * z - 0.01) && r.right <= (this.geo.client - right) * z + 0.01;
    }
    /** The scroll that brings tab `i` in from the right, to just clear of the right fade (the least it takes). */
    toRight(i: number): number {
        return Math.min(this.max, (i + 1) * this.geo.tabW - this.geo.client + FADE);
    }
    /** The scroll that brings tab `i` in from the left, to just clear of the left fade. */
    toLeft(i: number): number {
        return Math.max(0, i * this.geo.tabW - FADE);
    }
    get calls(): number {
        return this.writes.length;
    }
}

// Animation frames, run on demand.
let frames = new Map<number, FrameRequestCallback>();
let nextFrame = 1;
let clock = 0;
/** Runs the pending frames, a round at a time, the clock 40 ms further each round: a tween (160 ms) takes about six. */
function flush(max = 24): void {
    for (let i = 0; i < max && frames.size; i++) {
        const batch = [...frames.values()];
        frames = new Map();
        clock += 40;
        for (const cb of batch) cb(clock);
    }
}

// A ResizeObserver whose callbacks the test fires.
let observers: { cb: () => void; targets: Set<Element> }[] = [];
class FakeResizeObserver {
    private o = { cb: () => this.callback([], this as unknown as ResizeObserver), targets: new Set<Element>() };
    constructor(private callback: ResizeObserverCallback) {
        observers.push(this.o);
    }
    observe(el: Element) {
        this.o.targets.add(el);
    }
    unobserve(el: Element) {
        this.o.targets.delete(el);
    }
    disconnect() {
        observers = observers.filter((o) => o !== this.o);
    }
}
const resizeAll = () => observers.slice().forEach((o) => o.cb());

const models = new Map<HTMLElement, StripModel>();
let rectSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
    frames = new Map();
    nextFrame = 1;
    clock = 0;
    observers = [];
    reduceTransition = false;
    motionFactor = 1;
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        frames.set(nextFrame, cb);
        return nextFrame++;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    rectSpy = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
        const own = models.get(this);
        if (own) {
            own.reads++;
            const w = own.geo.client * own.geo.zoom;
            return { left: 0, right: w, width: w, top: 0, bottom: 20, height: 20 } as DOMRect;
        }
        const strip = this.closest<HTMLElement>(".pane-tab-strip");
        const model = strip && models.get(strip);
        if (!model) return { left: 0, right: 0, width: 0, top: 0, bottom: 0, height: 0 } as DOMRect;
        model.reads++;
        const r = model.rect(this);
        return { ...r, width: r.right - r.left, top: 0, bottom: 20, height: 20 } as DOMRect;
    });
});

afterEach(() => {
    cleanup();
    models.clear();
    rectSpy.mockRestore();
    vi.unstubAllGlobals();
    vi.useRealTimers();
});

const idsOf = (n: number) => Array.from({ length: n }, (_, i) => `t${i}`);

interface PaneProps {
    name: string;
    ids: Accessor<string[]>;
    paneActive: Accessor<string>;
    docActive: Accessor<string>;
    animateDoc?: boolean;
    onRoot?: (el: HTMLElement) => void;
}

/** What PaneChrome is to its strips: the root they register under, with the interaction listeners. */
function PaneUnderTest(props: PaneProps) {
    let root!: HTMLDivElement;
    onMount(() => {
        props.onRoot?.(root);
        onCleanup(installPaneInteractionReveal(root));
    });
    const common = {
        getId: (id: string) => id,
        getLabel: (id: string) => id,
        onActivate: () => {},
    };
    return (
        <div data-role="pane" data-name={props.name} ref={root}>
            <div class="header-strip">
                <PaneTabStrip tabs={props.ids()} activeId={props.paneActive()} {...common} />
            </div>
            <div class="doc-strip">
                <PaneTabStrip
                    tabs={props.ids()}
                    activeId={props.docActive()}
                    animateWidth={props.animateDoc}
                    {...common}
                    docDrag={{ docType: "editor", blockId: `blk-${props.name}`, onReorder: () => true }}
                />
            </div>
            <textarea class="content" />
        </div>
    );
}

interface Mounted {
    root: HTMLElement;
    content: HTMLElement;
    strips: Record<Kind, StripModel>;
    geo: Record<Kind, Geometry>;
    setActive: Record<Kind, (id: string) => void>;
    setCount: (n: number) => void;
    /** The tab closes and `next` takes its place as the selected one, in both strips. */
    close: (id: string, next: string) => void;
    setDisplayed: (v: boolean) => void;
}

/** A pane of two strips, stubbed. `settle` runs the mount reveal and lets the scroll edges catch up. */
function mountPane(opts: { name?: string; n?: number; active?: string; animateDoc?: boolean } = {}): Mounted {
    const name = opts.name ?? "a";
    const n = opts.n ?? 20;
    const [ids, setIds] = createSignal(idsOf(n));
    const [paneActive, setPaneActive] = createSignal(opts.active ?? "t10");
    const [docActive, setDocActive] = createSignal(opts.active ?? "t10");
    const [displayed, setDisplayed] = createSignal(true);
    let root!: HTMLElement;
    const { container } = render(() => (
        <WindowTabDisplayedProvider value={displayed}>
            <PaneUnderTest
                name={name}
                ids={ids}
                paneActive={paneActive}
                docActive={docActive}
                animateDoc={opts.animateDoc}
                onRoot={(el) => (root = el)}
            />
        </WindowTabDisplayedProvider>
    ));
    void container;
    const geo = (): Geometry => ({ tabs: n, tabW: TAB_W, client: CLIENT_W, zoom: 1 });
    const geos = { pane: geo(), doc: geo() };
    const strips = {
        pane: new StripModel(root.querySelector<HTMLElement>(".header-strip .pane-tab-strip")!, geos.pane),
        doc: new StripModel(root.querySelector<HTMLElement>(".doc-strip .pane-tab-strip")!, geos.doc),
    };
    models.set(strips.pane.el, strips.pane);
    models.set(strips.doc.el, strips.doc);
    const m: Mounted = {
        root,
        content: root.querySelector<HTMLElement>(".content")!,
        strips,
        geo: geos,
        setActive: { pane: setPaneActive, doc: setDocActive },
        setCount: (c) => {
            geos.pane.tabs = c;
            geos.doc.tabs = c;
            setIds(idsOf(c));
        },
        close: (id, next) => {
            geos.pane.tabs--;
            geos.doc.tabs--;
            batch(() => {
                setIds((prev) => prev.filter((t) => t !== id));
                setPaneActive(next);
                setDocActive(next);
            });
        },
        setDisplayed,
    };
    settle(m);
    return m;
}

/** Let the scroll edges measure the stubbed layout, and run every pending reveal. */
function settle(m: Mounted): void {
    for (const k of ["pane", "doc"] as const) m.strips[k].el.dispatchEvent(new Event("scroll"));
    flush();
}

/** The user wheels the strip by `dy` px. */
function userScroll(m: Mounted, kind: Kind, dy: number): void {
    // A trackpad-sized delta moves the strip by a tenth of it (strip-wheel.ts).
    fireEvent.wheel(m.strips[kind].el, { deltaY: dy * 10 });
    settle(m);
}

const KEY_TARGET = (m: Mounted) => m.content;

describe.each<Kind>(["pane", "doc"])("the %s tab strip reveals its selected tab", (kind) => {
    it("on mount, instantly, by the least it takes to bring the selected tab clear of the right fade", () => {
        const m = mountPane();
        const s = m.strips[kind];
        expect(s.isRevealed("t10")).toBe(true);
        expect(s.writes).toEqual([s.toRight(10)]);
        // Not aligned to the left edge: it sits where the scroll first brought it into view.
        expect(s.rect(s.tab("t10")).left).toBe(CLIENT_W - FADE - TAB_W);
    });

    it("when the selected tab changes, smoothly, by the least it takes", () => {
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        expect(s.value).toBe(0);
        s.writes.length = 0;
        m.setActive[kind]("t6");
        flush(3);
        // Under way: moved, not arrived (a tween, not a jump).
        expect(s.value).toBeGreaterThan(0);
        expect(s.value).toBeLessThan(s.toRight(6));
        flush();
        expect(s.value).toBe(s.toRight(6));
        expect(s.writes.length).toBeGreaterThan(2);
        expect(s.isRevealed("t6")).toBe(true);
        expect(s.rect(s.tab("t6")).left).toBe(CLIENT_W - FADE - TAB_W);
    });

    it("a tab off the left comes back to just clear of the left fade, not to the edge", () => {
        const m = mountPane({ active: "t10" });
        const s = m.strips[kind];
        m.setActive[kind]("t5");
        flush();
        expect(s.value).toBe(s.toLeft(5));
        expect(s.rect(s.tab("t5")).left).toBe(FADE);
        expect(s.isRevealed("t5")).toBe(true);
    });

    it("moves by exactly the overflow when the selected tab is only partly out of view", () => {
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        m.setActive[kind]("t3");
        flush();
        const from = s.value;
        s.writes.length = 0;
        // t4 starts at the right fade (372) and ends 100px past it: it needs exactly its own width more.
        m.setActive[kind]("t4");
        flush();
        expect(s.value).toBe(from + TAB_W);
        expect(s.value).toBe(s.toRight(4));
    });

    it("scrolls to the very end for the last tab, so the add button shows, and to 0 for the first", () => {
        const m = mountPane();
        const s = m.strips[kind];
        m.setActive[kind]("t19");
        flush();
        expect(s.value).toBe(s.max);
        m.setActive[kind]("t0");
        flush();
        expect(s.value).toBe(0);
    });

    it("leaves a selected tab that is already fully visible where it is, and so does a key in the pane", () => {
        const m = mountPane({ active: "t1" });
        const s = m.strips[kind];
        const before = s.calls;
        m.setActive[kind]("t2");
        flush();
        // t2 was in view beside t1: no scroll at all, not even a nudge.
        expect(s.calls).toBe(before);
        fireEvent.keyDown(KEY_TARGET(m), { key: "a" });
        fireEvent.pointerDown(m.content);
        flush();
        expect(s.calls).toBe(before);
        expect(s.value).toBe(0);
    });

    it("selecting the tabs near the end moves by the least each time, and the last one goes to the end so the add button shows", () => {
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        for (const id of ["t17", "t18"]) {
            m.setActive[kind](id);
            flush();
            expect(s.value).toBe(s.toRight(Number(id.slice(1))));
            expect(s.value).toBeLessThan(s.max);
            expect(s.isRevealed(id)).toBe(true);
        }
        m.setActive[kind]("t19");
        flush();
        expect(s.value).toBe(s.max);
        expect(s.isRevealed("t19")).toBe(true);
    });

    it("the pane becoming focused leaves a selected tab that is in view alone, and brings back one that is not", () => {
        const m = mountPane({ active: "t1" });
        const s = m.strips[kind];
        const before = s.calls;
        revealPaneStrips(m.root, "focus");
        flush();
        expect(s.calls).toBe(before);
        // Scrolled away by the user, so t1 sits under the right fade...
        m.setActive[kind]("t3");
        flush();
        userScroll(m, kind, -10);
        expect(s.isRevealed("t3")).toBe(false);
        s.writes.length = 0;
        revealPaneStrips(m.root, "focus");
        flush();
        // ...and focus brings it back by the little it takes.
        expect(s.value).toBe(s.toRight(3));
        expect(s.isRevealed("t3")).toBe(true);
        s.writes.length = 0;
        fireEvent.pointerDown(m.content);
        flush();
        expect(s.writes).toEqual([]);
    });

    it.each([
        ["a click", (m: Mounted) => fireEvent.pointerDown(m.content)],
        ["a right click", (m: Mounted) => fireEvent.pointerDown(m.content, { button: 2 })],
        ["a keystroke", (m: Mounted) => fireEvent.keyDown(m.content, { key: "a" })],
        ["Backspace", (m: Mounted) => fireEvent.keyDown(m.content, { key: "Backspace" })],
        ["Enter", (m: Mounted) => fireEvent.keyDown(m.content, { key: "Enter" })],
        ["a shortcut", (m: Mounted) => fireEvent.keyDown(m.content, { key: "k", ctrlKey: true })],
        ["a paste", (m: Mounted) => fireEvent.paste(m.content)],
        ["an IME composition", (m: Mounted) => fireEvent.compositionStart(m.content)],
        ["beforeinput", (m: Mounted) => fireEvent(m.content, new InputEvent("beforeinput", { bubbles: true }))],
        ["input", (m: Mounted) => fireEvent.input(m.content)],
    ])("again on %s in the pane, after the user scrolled it away", (_name, act) => {
        const m = mountPane();
        const s = m.strips[kind];
        userScroll(m, kind, -2000);
        expect(s.value).toBe(0);
        expect(s.isRevealed("t10")).toBe(false);
        s.writes.length = 0;
        act(m);
        flush();
        // Smoothly (several steps), by the least it takes.
        expect(s.writes.length).toBeGreaterThan(2);
        expect(s.value).toBe(s.toRight(10));
        expect(s.isRevealed("t10")).toBe(true);
    });

    it.each(["Shift", "Control", "Alt", "Meta", "CapsLock"])("not on a lone %s", (key) => {
        const m = mountPane();
        const s = m.strips[kind];
        userScroll(m, kind, -2000);
        const before = s.calls;
        const reads = s.reads;
        fireEvent.keyDown(m.content, { key });
        flush();
        expect(s.calls).toBe(before);
        expect(s.reads).toBe(reads);
        expect(s.value).toBe(0);
    });

    it("not when the user only scrolls it away: it stays away, whatever happens around it", () => {
        const m = mountPane();
        const s = m.strips[kind];
        userScroll(m, kind, -2000);
        const before = s.calls;
        // Hover, a layout change (the strip resizes), another pane's change, scrolling the pane's content.
        fireEvent.pointerEnter(s.el);
        fireEvent.mouseOver(s.tab("t3"));
        resizeAll();
        const other = mountPane({ name: "b", active: "t2" });
        other.setActive[kind]("t15");
        fireEvent.scroll(m.content);
        flush();
        expect(s.value).toBe(0);
        expect(s.calls).toBe(before);
        // The next click or keystroke is the trigger.
        fireEvent.keyDown(m.content, { key: "x" });
        flush();
        expect(s.isRevealed("t10")).toBe(true);
    });

    it("on a layout change under the selected tab, if the user has not scrolled the strip", () => {
        const m = mountPane();
        const s = m.strips[kind];
        // A narrower strip pushes the selected tab out of view; nothing the user did.
        s.writes.length = 0;
        m.geo[kind].client = 300;
        resizeAll();
        flush();
        expect(s.isRevealed("t10")).toBe(true);
        // At once, by the least it takes: a resize is not a gesture to animate.
        expect(s.writes).toEqual([s.toRight(10)]);
    });

    it("a wheel while a smooth reveal is travelling stops it", () => {
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        m.setActive[kind]("t12");
        flush(4);
        const where = s.value;
        expect(where).toBeGreaterThan(0);
        expect(where).toBeLessThan(s.toRight(12));
        // A trackpad's horizontal swipe: the strip's own handler leaves it to the browser.
        fireEvent.wheel(s.el, { deltaX: 40, deltaY: 0 });
        flush();
        expect(s.value).toBe(where);
    });

    it("a wheel before the reveal's frame runs cancels it", () => {
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        s.writes.length = 0;
        m.setActive[kind]("t12");
        fireEvent.wheel(s.el, { deltaX: 40, deltaY: 0 });
        flush();
        expect(s.writes).toEqual([]);
    });

    it("selecting a tab that is already in view while a reveal travels stops it there", () => {
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        m.setActive[kind]("t12");
        flush(4);
        const where = s.value;
        expect(where).toBeGreaterThan(0);
        expect(where).toBeLessThan(s.toRight(12));
        // t10 (1000..1100) is in view through the rest of the tween: the strip shows 812..1212 now and 928 at the end.
        expect(s.isRevealed("t10")).toBe(true);
        m.setActive[kind]("t10");
        // The round that runs the new reveal also steps the old tween once; after it, nothing moves.
        flush(1);
        const stopped = s.value;
        expect(s.isRevealed("t10")).toBe(true);
        flush();
        expect(s.value).toBe(stopped);
        expect(s.value).toBeLessThan(s.toRight(12));
    });

    it("a selection right after a resize during a reveal still animates, instead of jumping", () => {
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        m.setActive[kind]("t12");
        flush(4);
        m.geo[kind].client = 300;
        resizeAll();
        s.writes.length = 0;
        m.setActive[kind]("t2");
        flush();
        expect(s.value).toBe(s.toLeft(2));
        // A tween of several steps, not a tween cut short by an instant write.
        expect(s.writes.length).toBeGreaterThan(3);
    });

    it("a pointer going down on the strip stops a reveal travelling", () => {
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        m.setActive[kind]("t12");
        flush(4);
        const where = s.value;
        expect(where).toBeLessThan(s.toRight(12));
        fireEvent.pointerDown(s.tab("t3"));
        flush();
        expect(s.value).toBe(where);
    });

    it("a strip with no overflow reads no layout for a click or a keystroke", () => {
        const m = mountPane({ n: 3, active: "t1" });
        const s = m.strips[kind];
        const reads = s.reads;
        fireEvent.pointerDown(m.content);
        fireEvent.keyDown(m.content, { key: "a" });
        fireEvent.paste(m.content);
        revealPaneStrips(m.root, "focus");
        flush();
        expect(s.reads).toBe(reads);
        expect(s.calls).toBe(0);
    });

    it("a hidden strip reads and writes no geometry, and reveals once when shown", () => {
        const m = mountPane();
        const s = m.strips[kind];
        userScroll(m, kind, -2000);
        m.setDisplayed(false);
        flush();
        const reads = s.reads;
        const calls = s.calls;
        m.setActive[kind]("t14");
        fireEvent.keyDown(m.content, { key: "a" });
        fireEvent.pointerDown(m.content);
        revealPaneStrips(m.root, "focus");
        flush();
        expect(s.reads).toBe(reads);
        expect(s.calls).toBe(calls);
        m.setDisplayed(true);
        flush();
        // Once, instantly: shown means laid out.
        expect(s.writes).toHaveLength(calls + 1);
        expect(s.isRevealed("t14")).toBe(true);
    });

    it("uses the instant write under reduced motion or with Animations off, and a tween otherwise", () => {
        reduceTransition = true;
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        s.writes.length = 0;
        m.setActive[kind]("t12");
        flush();
        expect(s.writes).toEqual([s.toRight(12)]);
        reduceTransition = false;
        motionFactor = 0;
        s.writes.length = 0;
        m.setActive[kind]("t2");
        flush();
        expect(s.writes).toEqual([s.toLeft(2)]);
        motionFactor = 1;
        s.writes.length = 0;
        m.setActive[kind]("t14");
        flush();
        expect(s.writes.length).toBeGreaterThan(2);
        expect(s.value).toBe(s.toRight(14));
    });

    it("the tween takes the designed time, scaled by the Animations setting", () => {
        motionFactor = 0.5;
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        m.setActive[kind]("t12");
        // 160 ms halved is 80 ms: with frames 40 ms apart it has landed two frames after it starts.
        flush(1);
        flush(1);
        flush(1);
        flush(1);
        expect(s.value).toBe(s.toRight(12));
    });

    it("a smooth reveal that finds the selected tab already in view holds off nothing", () => {
        const m = mountPane({ active: "t2" });
        const s = m.strips[kind];
        const mounted = s.calls;
        fireEvent.keyDown(m.content, { key: "a" });
        flush();
        expect(s.calls).toBe(mounted);
        // Narrower now: the selected tab is out of view, and nothing the user did.
        m.geo[kind].client = 250;
        resizeAll();
        flush();
        expect(s.isRevealed("t2")).toBe(true);
    });

    it("a resize while a smooth reveal travels is looked at again when it ends", () => {
        const m = mountPane({ active: "t0" });
        const s = m.strips[kind];
        m.setActive[kind]("t12");
        flush(4);
        expect(s.value).toBeLessThan(s.toRight(12));
        m.geo[kind].client = 300;
        resizeAll();
        flush();
        expect(s.isRevealed("t12")).toBe(true);
    });

    it("a tab before the selected one closing moves it out of view: revealed again", () => {
        const m = mountPane();
        const s = m.strips[kind];
        // Scrolled (not by the user's wheel) so the selected tab sits just clear of the left fade.
        s.set(890);
        expect(s.isRevealed("t10")).toBe(true);
        m.close("t3", "t10");
        flush();
        expect(s.isRevealed("t10")).toBe(true);
    });

    it("watches every tab's width, so renaming an earlier one is a layout change too", () => {
        const m = mountPane();
        const s = m.strips[kind];
        expect(observers.some((o) => o.targets.has(s.tab("t3")))).toBe(true);
        // A wider earlier tab pushes the selected one out of view.
        m.geo[kind].tabW = 150;
        resizeAll();
        flush();
        expect(s.isRevealed("t10")).toBe(true);
    });

    it("a wheel or a touch over a strip that cannot scroll is not the user scrolling it", () => {
        const m = mountPane({ n: 3, active: "t2" });
        const s = m.strips[kind];
        fireEvent.wheel(s.el, { deltaY: 50 });
        fireEvent.touchMove(s.el);
        // Tabs arrive and the strip narrows until it overflows and hides the selected one.
        m.setCount(20);
        m.geo[kind].client = 250;
        resizeAll();
        flush();
        expect(s.isRevealed("t2")).toBe(true);
    });

    it("checks once for a burst of events in one frame", () => {
        const m = mountPane();
        const s = m.strips[kind];
        userScroll(m, kind, -2000);
        s.writes.length = 0;
        for (let i = 0; i < 8; i++) fireEvent.keyDown(m.content, { key: "a" });
        fireEvent.pointerDown(m.content);
        flush();
        // One tween (about five steps), not eight restarted ones.
        expect(s.writes.length).toBeGreaterThan(1);
        expect(s.writes.length).toBeLessThanOrEqual(6);
        expect(s.value).toBe(s.toRight(10));
    });
});

describe("a pane's two strips and other panes", () => {
    it("the strips reveal independently", () => {
        const m = mountPane();
        userScroll(m, "doc", -2000);
        m.strips.pane.writes.length = 0;
        m.strips.doc.writes.length = 0;
        fireEvent.keyDown(m.content, { key: "a" });
        flush();
        // Only the document tab strip was away; the pane tab strip had nothing to do.
        expect(m.strips.doc.writes.length).toBeGreaterThan(0);
        expect(m.strips.pane.calls).toBe(0);
    });

    it.each<[Kind, Kind]>([
        ["pane", "doc"],
        ["doc", "pane"],
    ])("an event inside the %s tab strip does not reveal it, but reveals the %s tab strip", (inside, other) => {
        const m = mountPane();
        userScroll(m, "pane", -2000);
        userScroll(m, "doc", -2000);
        for (const k of ["pane", "doc"] as const) m.strips[k].writes.length = 0;
        fireEvent.keyDown(m.strips[inside].tab("t2"), { key: "a" });
        fireEvent.pointerDown(m.strips[inside].tab("t2"));
        flush();
        expect(m.strips[inside].writes).toEqual([]);
        expect(m.strips[inside].value).toBe(0);
        expect(m.strips[other].writes.length).toBeGreaterThan(0);
    });

    it("events in another pane do not reach this pane's strips", () => {
        const a = mountPane({ name: "a" });
        const b = mountPane({ name: "b" });
        for (const m of [a, b]) for (const k of ["pane", "doc"] as const) userScroll(m, k, -2000);
        for (const m of [a, b]) for (const k of ["pane", "doc"] as const) m.strips[k].writes.length = 0;
        fireEvent.keyDown(b.content, { key: "a" });
        fireEvent.pointerDown(b.content);
        flush();
        for (const k of ["pane", "doc"] as const) {
            expect(a.strips[k].writes).toEqual([]);
            expect(b.strips[k].writes.length).toBeGreaterThan(0);
        }
    });

    it("the pane becoming focused reveals both strips, and only that pane's", () => {
        const a = mountPane({ name: "a" });
        const b = mountPane({ name: "b" });
        for (const m of [a, b]) for (const k of ["pane", "doc"] as const) userScroll(m, k, -2000);
        for (const m of [a, b]) for (const k of ["pane", "doc"] as const) m.strips[k].writes.length = 0;
        revealPaneStrips(a.root, "focus");
        flush();
        for (const k of ["pane", "doc"] as const) {
            expect(a.strips[k].writes.length).toBeGreaterThan(0);
            expect(b.strips[k].writes).toEqual([]);
        }
    });
});

describe("a document block that is kept alive and not showing", () => {
    it("reads no geometry until it is shown", () => {
        const m = mountPane({ name: "kept" });
        const s = m.strips.doc;
        userScroll(m, "doc", -2000);
        setKeepAliveBlockDormant("blk-kept", true);
        flush();
        const reads = s.reads;
        m.setActive.doc("t15");
        fireEvent.keyDown(m.content, { key: "a" });
        flush();
        expect(s.reads).toBe(reads);
        setKeepAliveBlockDormant("blk-kept", false);
        flush();
        expect(s.isRevealed("t15")).toBe(true);
    });
});

describe("animations on the strip", () => {
    it("measures again once the width transition has ended", () => {
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
        const m = mountPane({ animateDoc: true, active: "t0" });
        const s = m.strips.doc;
        // A tab is added and selected: the strip's width animates to its new size.
        m.geo.doc.client = 460;
        s.writes.length = 0;
        m.setCount(21);
        m.setActive.doc("t20");
        flush();
        const started = s.writes.length;
        expect(started).toBeGreaterThan(0);
        // Tab widths change during the transition, so the first target was for sizes that are gone.
        m.geo.doc.tabW = 120;
        resizeAll();
        flush();
        expect(s.writes).toHaveLength(started);
        vi.advanceTimersByTime(400);
        flush();
        expect(s.value).toBe(s.max);
        expect(s.isRevealed("t20")).toBe(true);
    });

    it("waits out the landing cue of the tab that took the place of a closed one", () => {
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
        const m = mountPane({ active: "t10" });
        const s = m.strips.doc;
        userScroll(m, "doc", -2000);
        s.writes.length = 0;
        // The document tab in front closes, and t11 is promoted.
        m.close("t10", "t11");
        flush();
        // The landing bounce moves the tab's rect: measure after it.
        expect(s.tab("t11").classList.contains("pane-tab--landing")).toBe(true);
        expect(s.writes).toEqual([]);
        vi.advanceTimersByTime(450);
        flush();
        expect(s.writes.length).toBeGreaterThan(0);
        expect(s.isRevealed("t11")).toBe(true);
    });
});

describe("zoom", () => {
    it.each([0.5, 1, 2])("reveals at chrome zoom %s", (zoom) => {
        const m = mountPane({ active: "t0" });
        const s = m.strips.pane;
        m.geo.pane.zoom = zoom;
        m.setActive.pane("t9");
        flush();
        expect(s.isRevealed("t9")).toBe(true);
        // The scroll is in the strip's own pixels, so chrome zoom does not change it.
        expect(s.value).toBe(s.toRight(9));
    });
});
