// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The controller that keeps a strip's selected tab in view (strip-reveal-controller.ts), on its own:
 * which request becomes a reveal, when it runs, what it asks `revealInStrip` for, and how it holds
 * back for a landing tab, a smooth reveal in flight, a width transition, the user's own scroll and a
 * hidden pane. The geometry that decides the scroll has its own tests (strip-reveal.test.ts, and
 * PaneTabStrip.reveal.test.tsx through a whole strip); here `revealInStrip` is a spy.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DURATION_BASE_MS } from "@/app/motion/tokens";
import { revealPaneStrips } from "./pane-strip-registry";
import { cancelStripScroll, revealInStrip, STRIP_EDGE_FADE_PX } from "./strip-reveal";
import { createStripReveal, registerStripWithPane, type StripReveal } from "./strip-reveal-controller";

vi.mock("./strip-reveal", async (importOriginal) => ({
    ...(await importOriginal<typeof import("./strip-reveal")>()),
    revealInStrip: vi.fn(() => false),
    cancelStripScroll: vi.fn(() => false),
}));

// The motion scale and the categories that run their reduced treatment, set per test.
let scale = 2;
let reduced: string[] = [];
vi.mock("@/app/motion/motion", () => ({
    scaledMs: (ms: number) => ms * scale,
    shouldReduceMotion: (category: string) => reduced.includes(category),
}));

// Animation frames, run on demand.
let frames = new Map<number, FrameRequestCallback>();
let nextFrame = 1;
function flush(): void {
    const batch = [...frames.values()];
    frames = new Map();
    for (const cb of batch) cb(0);
}

// A ResizeObserver whose callbacks the test fires.
class FakeResizeObserver {
    static all: FakeResizeObserver[] = [];
    observed = new Set<Element>();
    disconnected = false;
    constructor(readonly callback: () => void) {
        FakeResizeObserver.all.push(this);
    }
    observe(el: Element) {
        this.observed.add(el);
    }
    unobserve(el: Element) {
        this.observed.delete(el);
    }
    disconnect() {
        this.disconnected = true;
    }
}

const reveals = vi.mocked(revealInStrip);
const cancels = vi.mocked(cancelStripScroll);

interface Options {
    active?: string | null;
    ids?: string[];
    shown?: boolean;
    overflows?: boolean;
    noStrip?: boolean;
    /** Tabs carrying this class (landing or dragging) when the strip is built. */
    marked?: { id: string; cls: string };
    /** Run the mount reveal and forget it. */
    settle?: boolean;
}

function setup(options: Options = {}) {
    const ids = options.ids ?? ["a", "b", "c"];
    const strip = document.createElement("div");
    document.body.appendChild(strip);
    const tabEls = new Map<string, HTMLElement>();
    const addTab = (id: string) => {
        const el = document.createElement("div");
        el.dataset.tabId = id;
        el.className = "pane-tab";
        strip.appendChild(el);
        tabEls.set(id, el);
        return el;
    };
    for (const id of ids) addTab(id);
    if (options.marked) tabEls.get(options.marked.id)!.classList.add(options.marked.cls);
    const [active, setActive] = createSignal<string | null>(options.active === undefined ? "b" : options.active);
    const [tabIds, setTabIds] = createSignal(ids);
    const [shown, setShown] = createSignal(options.shown ?? true);
    const [overflows, setOverflows] = createSignal(options.overflows ?? true);
    let reveal!: StripReveal;
    let dispose!: () => void;
    createRoot((d) => {
        dispose = d;
        reveal = createStripReveal({
            strip: () => (options.noStrip ? undefined : strip),
            activeId: active,
            tabIds,
            shown,
            overflows,
        });
    });
    disposers.push(dispose);
    if (options.settle ?? true) {
        flush();
        reveals.mockClear();
    }
    return { strip, tabEls, addTab, reveal, dispose, setActive, setTabIds, setShown, setOverflows };
}

/** The `onSettled` the last reveal was given: the tween calls it when it ends. */
const settled = () => (reveals.mock.calls.at(-1)![3] as { onSettled: () => void }).onSettled;
const behavior = () => (reveals.mock.calls.at(-1)![3] as { behavior: string }).behavior;

beforeEach(() => {
    scale = 2;
    reduced = [];
    frames = new Map();
    nextFrame = 1;
    FakeResizeObserver.all = [];
    reveals.mockReset();
    reveals.mockReturnValue(false);
    cancels.mockReset();
    cancels.mockReturnValue(false);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        frames.set(nextFrame, cb);
        return nextFrame++;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => void frames.delete(id));
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
});

afterEach(() => {
    cleanupAll();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
});

describe("createStripReveal — what a reveal asks for", () => {
    it("reveals the selected tab once on mount, instantly, with the strip's options", () => {
        const { strip, tabEls } = setup({ settle: false });
        expect(reveals).not.toHaveBeenCalled();
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
        const [stripArg, itemArg, isLast, options] = reveals.mock.calls[0];
        expect(stripArg).toBe(strip);
        expect(itemArg).toBe(tabEls.get("b"));
        expect(isLast).toBe(false);
        expect(options).toEqual({
            isFirst: false,
            edgeClearance: STRIP_EDGE_FADE_PX,
            behavior: "instant",
            durationMs: DURATION_BASE_MS * 2,
            onSettled: expect.any(Function),
        });
    });

    it.each([
        ["a", true, false],
        ["b", false, false],
        ["c", false, true],
    ])("tells it whether tab %s is the first (%s) and the last (%s)", (active, first, last) => {
        setup({ active, settle: false });
        flush();
        const [, , isLast, options] = reveals.mock.calls[0];
        expect(isLast).toBe(last);
        expect((options as { isFirst: boolean }).isFirst).toBe(first);
    });

    it("queues one frame however many requests come before it runs", () => {
        const { reveal } = setup();
        reveal.request("selected");
        reveal.request("layout");
        reveal.request("selected");
        expect(frames.size).toBe(1);
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
    });

    it("reveals the tab that becomes selected", () => {
        const { setActive, tabEls } = setup();
        setActive("c");
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
        expect(reveals.mock.calls[0][1]).toBe(tabEls.get("c"));
    });

    it("has nothing to reveal without a strip, a selected tab, or that tab in the strip", () => {
        setup({ noStrip: true, settle: false });
        flush();
        expect(reveals).not.toHaveBeenCalled();
        cleanupAll();

        setup({ active: "zzz", settle: false });
        flush();
        expect(reveals).not.toHaveBeenCalled();
        cleanupAll();

        // With no selected tab it does not even wait for a landing tab.
        setup({ active: null, marked: { id: "a", cls: "pane-tab--landing" }, settle: false });
        flush();
        expect(reveals).not.toHaveBeenCalled();
        expect(vi.getTimerCount()).toBe(0);
    });

    it("schedules nothing while hidden, and does not reveal if it is hidden when the frame runs", () => {
        const { reveal, setShown } = setup({ shown: false, settle: false });
        expect(frames.size).toBe(0);
        reveal.request("selected");
        expect(frames.size).toBe(0);
        setShown(true);
        expect(frames.size).toBe(1);
        setShown(false);
        flush();
        expect(reveals).not.toHaveBeenCalled();
    });

    it("reveals again, instantly, when a hidden pane is shown again", () => {
        const { setShown } = setup();
        setShown(false);
        setShown(true);
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
        expect(behavior()).toBe("instant");
    });
});

describe("createStripReveal — smooth or instant", () => {
    it.each([
        ["selected", "smooth"],
        ["focus", "smooth"],
        ["interaction", "smooth"],
        ["layout", "instant"],
        ["visible", "instant"],
    ] as const)("a %s request reveals %s", (reason, expected) => {
        const { reveal } = setup();
        reveal.request(reason);
        flush();
        expect(behavior()).toBe(expected);
    });

    it("uses a smooth reveal for one request only", () => {
        const { reveal } = setup();
        reveal.request("selected");
        flush();
        expect(behavior()).toBe("smooth");
        reveal.request("visible");
        flush();
        expect(behavior()).toBe("instant");
    });

    it("is instant when transitions run their reduced treatment, and still smooth for the other categories", () => {
        reduced = ["transition"];
        const { reveal } = setup();
        reveal.request("selected");
        flush();
        expect(behavior()).toBe("instant");
        reduced = ["feedback", "status", "decorative"];
        reveal.request("selected");
        flush();
        expect(behavior()).toBe("smooth");
    });

    it("is instant when the motion scale is 0, with a duration of 0", () => {
        scale = 0;
        const { reveal } = setup();
        reveal.request("selected");
        flush();
        expect(behavior()).toBe("instant");
        expect((reveals.mock.calls.at(-1)![3] as { durationMs: number }).durationMs).toBe(0);
    });

    it("does nothing for an interaction or a focus while the tabs do not overflow, but still for a selection", () => {
        const { reveal, setOverflows } = setup({ overflows: false });
        reveal.request("interaction");
        reveal.request("focus");
        expect(frames.size).toBe(0);
        reveal.request("selected");
        expect(frames.size).toBe(1);
        flush();
        setOverflows(true);
        reveal.request("interaction");
        expect(frames.size).toBe(1);
    });
});

describe("createStripReveal — a landing or dragged tab", () => {
    it.each(["pane-tab--landing", "pane-tab--dragging"])("waits out a tab marked %s, then reveals", (cls) => {
        setup({ marked: { id: "a", cls }, settle: false });
        flush();
        expect(reveals).not.toHaveBeenCalled();
        expect(frames.size).toBe(0);
        vi.advanceTimersByTime(839);
        expect(frames.size).toBe(0);
        vi.advanceTimersByTime(1);
        expect(frames.size).toBe(1);
        flush();
        // It does not wait a second time, though the tab is still marked.
        expect(reveals).toHaveBeenCalledTimes(1);
    });

    it("keeps a smooth request through the wait", () => {
        const { setActive, tabEls } = setup();
        tabEls.get("a")!.classList.add("pane-tab--landing");
        setActive("c");
        flush();
        expect(reveals).not.toHaveBeenCalled();
        vi.advanceTimersByTime(840);
        flush();
        expect(behavior()).toBe("smooth");
    });

    it("waits again for the next reveal if a tab is landing then too", () => {
        const { reveal, tabEls } = setup();
        tabEls.get("a")!.classList.add("pane-tab--landing");
        reveal.request("selected");
        flush();
        vi.advanceTimersByTime(840);
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
        reveal.request("selected");
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(840);
        flush();
        expect(reveals).toHaveBeenCalledTimes(2);
    });
});

describe("createStripReveal — the user takes over", () => {
    it.each(["noteUserScroll", "notePointerDown"] as const)("%s drops a reveal that is queued or waiting", (method) => {
        const { reveal, strip, tabEls } = setup();
        reveal.request("selected");
        expect(frames.size).toBe(1);
        reveal[method]();
        expect(frames.size).toBe(0);
        expect(cancels).toHaveBeenCalledWith(strip);
        flush();
        expect(reveals).not.toHaveBeenCalled();

        // A request after that is queued again, and the wait of a landing tab ends with it.
        reveal.request("selected");
        expect(frames.size).toBe(1);
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
        reveals.mockClear();
        tabEls.get("a")!.classList.add("pane-tab--landing");
        reveal.request("selected");
        flush();
        expect(vi.getTimerCount()).toBe(1);
        reveal[method]();
        expect(vi.getTimerCount()).toBe(0);
        vi.advanceTimersByTime(2000);
        expect(frames.size).toBe(0);
    });

    it("waits for a landing tab again after the user took over in the middle of a wait", () => {
        const { reveal, tabEls } = setup();
        tabEls.get("a")!.classList.add("pane-tab--landing");
        reveal.request("selected");
        flush();
        reveal.noteUserScroll();
        reveal.request("selected");
        flush();
        expect(reveals).not.toHaveBeenCalled();
        vi.advanceTimersByTime(840);
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
    });

    it("ignores a layout change after the user scrolled, until the next selection", () => {
        const { reveal, setActive } = setup();
        reveal.noteUserScroll();
        reveal.request("layout");
        expect(frames.size).toBe(0);
        setActive("c");
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
        reveal.request("layout");
        expect(frames.size).toBe(1);
    });

    it("counts a scroll as the user's only while a pointer is held on the strip", () => {
        const { reveal, strip } = setup();
        strip.dispatchEvent(new Event("scroll"));
        reveal.request("layout");
        expect(frames.size).toBe(1);
        flush();

        reveal.notePointerDown();
        strip.dispatchEvent(new Event("scroll"));
        reveal.request("layout");
        expect(frames.size).toBe(0);
    });

    it.each(["pointerup", "pointercancel"])(
        "a %s anywhere in the window ends the hold, even where something stops it",
        (type) => {
            const { reveal, strip } = setup();
            const stopper = document.createElement("div");
            stopper.addEventListener(type, (e) => e.stopPropagation());
            document.body.appendChild(stopper);
            reveal.notePointerDown();
            stopper.dispatchEvent(new Event(type, { bubbles: true }));
            strip.dispatchEvent(new Event("scroll"));
            reveal.request("layout");
            expect(frames.size).toBe(1);
        }
    );
});

describe("createStripReveal — a smooth reveal in flight", () => {
    /** A selection that travels: the spy says the tween started. */
    function travelling() {
        const s = setup();
        reveals.mockReturnValueOnce(true);
        s.setActive("c");
        flush();
        expect(behavior()).toBe("smooth");
        return s;
    }

    it("holds a layout change back, and looks again when the tween ends", () => {
        const { reveal } = travelling();
        reveal.request("layout");
        expect(frames.size).toBe(0);
        settled()();
        expect(frames.size).toBe(1);
        flush();
        expect(reveals).toHaveBeenCalledTimes(2);
        expect(behavior()).toBe("instant");
    });

    it("looks again for a layout change only if one came", () => {
        travelling();
        settled()();
        expect(frames.size).toBe(0);
    });

    it("does not look again after the user scrolled", () => {
        const { reveal } = travelling();
        const done = settled();
        reveal.request("layout");
        reveal.noteUserScroll();
        expect(frames.size).toBe(0);
        done();
        expect(frames.size).toBe(0);
    });

    it("does not look again when a pointer was pressed on the strip", () => {
        const { reveal } = travelling();
        reveal.request("layout");
        reveal.notePointerDown();
        expect(frames.size).toBe(0);
    });

    it("takes a layout change again once a press on the strip has stopped the tween", () => {
        const { reveal } = travelling();
        reveal.notePointerDown();
        window.dispatchEvent(new Event("pointerup"));
        reveal.request("layout");
        expect(frames.size).toBe(1);
    });

    it("keeps a stale held-back layout change from reaching a later tween that began during a width transition", () => {
        reveals.mockReturnValue(true);
        const { reveal, setActive } = setup();
        reveal.setWidthAnimating(true);
        reveal.request("layout");
        setActive("c");
        flush();
        reveal.setWidthAnimating(false);
        flush();
        settled()();
        expect(frames.size).toBe(0);
    });

    it("leaves a layout change waiting for a width transition that is still running", () => {
        const { reveal } = travelling();
        reveal.request("layout");
        reveal.setWidthAnimating(true);
        settled()();
        expect(frames.size).toBe(0);
    });

    it("forgets a held-back layout change once a later reveal is not a tween", () => {
        const { reveal, setShown } = travelling();
        const first = settled();
        reveal.request("layout");
        setShown(false);
        setShown(true);
        flush();
        expect(behavior()).toBe("instant");
        first();
        expect(frames.size).toBe(0);
    });

    it("is not held back by an instant reveal that came before it", () => {
        const { setActive } = setup();
        // The mount reveal was instant and moved nothing; the tween that follows starts clean.
        reveals.mockReturnValueOnce(true);
        setActive("c");
        flush();
        settled()();
        expect(frames.size).toBe(0);
    });

    it("an instant reveal that did not scroll leaves nothing held back for the next tween", () => {
        const { reveal, setActive } = setup();
        reveal.request("layout");
        flush();
        reveals.mockReturnValueOnce(true);
        setActive("c");
        flush();
        settled()();
        expect(frames.size).toBe(0);
    });
});

describe("createStripReveal — a width transition", () => {
    it("holds a layout change until the transition ends, then looks once", () => {
        const { reveal } = setup();
        reveal.setWidthAnimating(true);
        reveal.request("layout");
        expect(frames.size).toBe(0);
        reveal.setWidthAnimating(true);
        expect(frames.size).toBe(0);
        reveal.setWidthAnimating(false);
        expect(frames.size).toBe(1);
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
        expect(behavior()).toBe("instant");
        reveal.setWidthAnimating(false);
        expect(frames.size).toBe(0);
    });

    it("looks again when it ends after a reveal that ran during it", () => {
        const { reveal } = setup();
        reveal.setWidthAnimating(true);
        reveal.request("selected");
        flush();
        expect(reveals).toHaveBeenCalledTimes(1);
        reveal.setWidthAnimating(false);
        expect(frames.size).toBe(1);
    });

    it("looks at nothing when it ends with nothing to look at", () => {
        const { reveal } = setup();
        reveal.setWidthAnimating(true);
        reveal.setWidthAnimating(false);
        expect(frames.size).toBe(0);
    });

    it("does not look when the user scrolled, or the pane is hidden, in the meantime", () => {
        const a = setup();
        a.reveal.setWidthAnimating(true);
        a.reveal.request("layout");
        a.reveal.noteUserScroll();
        a.reveal.setWidthAnimating(false);
        expect(frames.size).toBe(0);
        cleanupAll();

        const b = setup();
        b.reveal.setWidthAnimating(true);
        b.reveal.request("layout");
        b.setShown(false);
        b.reveal.setWidthAnimating(false);
        expect(frames.size).toBe(0);
    });

    it("follows a tween that is still travelling to the settled widths, and is instant when none is", () => {
        const a = setup();
        reveals.mockReturnValueOnce(true);
        a.setActive("c");
        flush();
        a.reveal.setWidthAnimating(true);
        a.reveal.request("layout");
        a.reveal.setWidthAnimating(false);
        flush();
        expect(behavior()).toBe("smooth");
        cleanupAll();
        reveals.mockClear();

        const b = setup();
        b.reveal.setWidthAnimating(true);
        b.reveal.request("layout");
        b.reveal.setWidthAnimating(false);
        flush();
        expect(behavior()).toBe("instant");
    });
});

describe("createStripReveal — the tabs of the strip", () => {
    it("watches the strip and every tab for a resize, and looks again on one", () => {
        const { strip, tabEls } = setup();
        const ro = FakeResizeObserver.all[0];
        expect(ro.observed).toEqual(new Set([strip, ...tabEls.values()]));
        ro.callback();
        expect(frames.size).toBe(1);
        flush();
        expect(behavior()).toBe("instant");
    });

    it("looks again when the tab list changes, instantly, and not when it comes back the same", () => {
        const { setTabIds } = setup();
        setTabIds(["a", "c", "b"]);
        expect(frames.size).toBe(1);
        flush();
        expect(behavior()).toBe("instant");
        setTabIds(["a", "c", "b"].slice());
        expect(frames.size).toBe(0);
    });

    it("tells two lists apart that join to the same text", () => {
        const { setTabIds } = setup({ ids: ["ab", "c"], active: "ab" });
        setTabIds(["a", "bc"]);
        expect(frames.size).toBe(1);
    });

    it("watches the tabs the strip has now, not the ones it had", () => {
        const { strip, tabEls, addTab, setTabIds } = setup();
        const ro = FakeResizeObserver.all[0];
        const old = tabEls.get("a")!;
        strip.removeChild(old);
        const added = addTab("d");
        setTabIds(["b", "c", "d"]);
        expect(ro.observed.has(old)).toBe(false);
        expect(ro.observed.has(added)).toBe(true);
        expect(ro.observed.has(strip)).toBe(true);
    });
});

describe("createStripReveal — teardown", () => {
    it("drops a queued reveal and a landing wait, and stops a tween on the strip", () => {
        const a = setup();
        a.reveal.request("selected");
        a.dispose();
        expect(frames.size).toBe(0);
        expect(cancels).toHaveBeenCalledWith(a.strip);
        cleanupAll();

        const b = setup({ marked: { id: "a", cls: "pane-tab--landing" }, settle: false });
        flush();
        expect(vi.getTimerCount()).toBe(1);
        b.dispose();
        expect(vi.getTimerCount()).toBe(0);
    });

    it("takes its listeners off the strip and the window, and stops observing", () => {
        const { strip, dispose } = setup();
        const onStrip = vi.spyOn(strip, "removeEventListener");
        const onWindow = vi.spyOn(window, "removeEventListener");
        const ro = FakeResizeObserver.all[0];
        dispose();
        expect(onStrip).toHaveBeenCalledWith("scroll", expect.any(Function));
        expect(onWindow).toHaveBeenCalledWith("pointerup", expect.any(Function), true);
        expect(onWindow).toHaveBeenCalledWith("pointercancel", expect.any(Function), true);
        expect(ro.disconnected).toBe(true);
    });

    it("has no listeners or observer to take off without a strip", () => {
        const { dispose } = setup({ noStrip: true });
        expect(FakeResizeObserver.all).toHaveLength(0);
        expect(() => dispose()).not.toThrow();
    });
});

describe("registerStripWithPane", () => {
    function pane() {
        const root = document.createElement("div");
        root.setAttribute("data-role", "pane");
        const strip = document.createElement("div");
        const inside = document.createElement("span");
        strip.appendChild(inside);
        const outside = document.createElement("span");
        root.append(strip, outside);
        document.body.appendChild(root);
        return { root, strip, inside, outside };
    }
    const fake = () => ({ request: vi.fn() }) as unknown as StripReveal & { request: ReturnType<typeof vi.fn> };

    it("lets the pane ask the strip to reveal, except for the strip that holds the origin", () => {
        const { root, strip, inside, outside } = pane();
        const reveal = fake();
        createRoot(() => registerStripWithPane(() => strip, reveal));
        revealPaneStrips(root, "focus");
        expect(reveal.request).toHaveBeenCalledTimes(1);
        expect(reveal.request).toHaveBeenLastCalledWith("focus");
        revealPaneStrips(root, "interaction", outside);
        expect(reveal.request).toHaveBeenCalledTimes(2);
        revealPaneStrips(root, "interaction", inside);
        expect(reveal.request).toHaveBeenCalledTimes(2);
    });

    it("is taken out of the pane when torn down", () => {
        const { root, strip } = pane();
        const reveal = fake();
        const dispose = createRoot((d) => {
            registerStripWithPane(() => strip, reveal);
            return d;
        });
        dispose();
        revealPaneStrips(root, "focus");
        expect(reveal.request).not.toHaveBeenCalled();
    });

    it("registers nothing without a strip", () => {
        expect(() => createRoot(() => registerStripWithPane(() => undefined, fake()))).not.toThrow();
    });
});

const disposers: (() => void)[] = [];
function cleanupAll(): void {
    while (disposers.length) disposers.pop()!();
    vi.clearAllTimers();
    document.body.innerHTML = "";
    frames = new Map();
    FakeResizeObserver.all = [];
    reveals.mockReset();
    reveals.mockReturnValue(false);
    cancels.mockReset();
    cancels.mockReturnValue(false);
}
