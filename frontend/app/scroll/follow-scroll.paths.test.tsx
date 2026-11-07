// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for createFollowScroll, the DOM binding of the follow state machine: how
 * each gesture (wheel, touch, pointer, key, selection) is classified and what it asks of the
 * reducer, what a size observation or a scroll batch returns and asks of the host (pin, load older,
 * fill, transition), the user-intent window and the held pointer and scrollbar, and the teardown.
 * They pin today's behaviour (written against the unsplit function) so it can be split into named
 * pieces without changing it. The reducer and the fill pass are the real ones; the host is a
 * recorder with a clock the tests move, and the events are dispatched on a real element in jsdom.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { FILL_MAX_PAGES } from "./fill-pass";
import { DETACHED, FOLLOWING, suspended, type FollowState, type ScrollGeometry } from "./follow-controller";
import { createFollowScroll, INTENT_WINDOW_MS, type FollowScroll, type FollowScrollHost } from "./follow-scroll";

type Transition = { from: FollowState; to: FollowState; cause: string; detail?: string };

type Fixture = {
    fs: FollowScroll;
    host: {
        pin: ReturnType<typeof vi.fn<(cause: string) => void>>;
        loadOlder: ReturnType<typeof vi.fn<(geo: ScrollGeometry) => void>>;
        fillOlder: ReturnType<typeof vi.fn<(geo: ScrollGeometry) => void>>;
        fillStalled: ReturnType<typeof vi.fn<(stalled: boolean) => void>>;
        released: ReturnType<typeof vi.fn<() => void>>;
    };
    transitions: Transition[];
    scroller: HTMLElement;
    child: HTMLElement;
    older: { has: boolean; loading: boolean };
    clock: { t: number };
};

let cleanups: (() => void)[] = [];

const geo = (scrollTop: number, scrollHeight = 2000, clientHeight = 500): ScrollGeometry => ({
    scrollTop,
    scrollHeight,
    clientHeight,
});

function setup(initial?: FollowState, withHostExtras = true): Fixture {
    const scroller = document.createElement("div");
    const child = document.createElement("p");
    child.textContent = "some transcript text";
    scroller.appendChild(child);
    document.body.appendChild(scroller);
    const transitions: Transition[] = [];
    const older = { has: false, loading: false };
    const clock = { t: 1000 };
    const host = {
        pin: vi.fn<(cause: string) => void>(),
        loadOlder: vi.fn<(geo: ScrollGeometry) => void>(),
        fillOlder: vi.fn<(geo: ScrollGeometry) => void>(),
        fillStalled: vi.fn<(stalled: boolean) => void>(),
        released: vi.fn<() => void>(),
    };
    const hostApi: FollowScrollHost = {
        pin: host.pin,
        loadOlder: host.loadOlder,
        transition: (from, to, cause, detail) => {
            transitions.push({ from, to, cause, detail });
        },
        hasOlderHistory: () => older.has,
        loadingOlder: () => older.loading,
        now: () => clock.t,
        ...(withHostExtras
            ? { fillOlder: host.fillOlder, fillStalled: host.fillStalled, released: host.released }
            : {}),
    };
    const fs = createFollowScroll(scroller, hostApi, initial);
    cleanups.push(() => {
        fs.dispose();
        scroller.remove();
    });
    return { fs, host, transitions, scroller, child, older, clock };
}

function wheel(target: EventTarget, init: WheelEventInit): WheelEvent {
    const e = new WheelEvent("wheel", { bubbles: true, cancelable: true, ...init });
    target.dispatchEvent(e);
    return e;
}

function mouse(type: string, target: EventTarget, init: MouseEventInit = {}): MouseEvent {
    const e = new MouseEvent(type, { bubbles: true, cancelable: true, ...init });
    target.dispatchEvent(e);
    return e;
}

function key(target: EventTarget, k: string, init: KeyboardEventInit = {}): KeyboardEvent {
    const e = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init });
    target.dispatchEvent(e);
    return e;
}

beforeEach(() => {
    cleanups = [];
});

afterEach(() => {
    for (const c of cleanups) c();
    document.getSelection()?.removeAllRanges();
    vi.restoreAllMocks();
});

describe("createFollowScroll state", () => {
    it("starts following, with no gesture window and nothing held", () => {
        const f = setup();
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.fs.following()).toBe(true);
        expect(f.fs.windowOpen()).toBe(false);
        expect(f.fs.pointerHeld()).toBe(false);
        expect(f.fs.scrollbarHeld()).toBe(false);
    });

    it("has no gesture window at the start of the clock either", () => {
        const f = setup();
        f.clock.t = 100;
        expect(f.fs.windowOpen()).toBe(false);
    });

    it("starts in the state it is given", () => {
        const f = setup(DETACHED);
        expect(f.fs.state()).toBe(DETACHED);
        expect(f.fs.following()).toBe(false);
    });

    it("counts a suspended pane's follow intent as following only if it was following", () => {
        expect(setup(suspended("FOLLOWING")).fs.following()).toBe(true);
        expect(setup(suspended("DETACHED")).fs.following()).toBe(false);
    });

    it("reads the clock from performance.now when the host gives none", () => {
        const now = vi.spyOn(performance, "now").mockReturnValue(5000);
        const scroller = document.createElement("div");
        document.body.appendChild(scroller);
        const fs = createFollowScroll(scroller, {
            pin: vi.fn(),
            loadOlder: vi.fn(),
            transition: vi.fn(),
            hasOlderHistory: () => false,
            loadingOlder: () => false,
        });
        cleanups.push(() => {
            fs.dispose();
            scroller.remove();
        });
        wheel(scroller, { deltaY: 10 });
        expect(fs.windowOpen()).toBe(true);
        now.mockReturnValue(5000 + INTENT_WINDOW_MS + 1);
        expect(fs.windowOpen()).toBe(false);
    });
});

describe("createFollowScroll observed", () => {
    it("suspends a pane that has no height, naming the observer and what it saw, and asks for no pin", () => {
        const f = setup();
        expect(f.fs.observed("viewport", geo(300, 1500, 0))).toBe(false);
        expect(f.fs.state()).toEqual(suspended("FOLLOWING"));
        expect(f.transitions).toEqual([
            {
                from: FOLLOWING,
                to: suspended("FOLLOWING"),
                cause: "zero-size",
                detail: "via=viewport ch=0 sh=1500 top=300",
            },
        ]);
        expect(f.host.pin).not.toHaveBeenCalled();
    });

    it("sizes a pane that is one pixel tall: only a height of 0 or less suspends it", () => {
        const f = setup();
        expect(f.fs.observed("content", geo(0, 1000, 1))).toBe(true);
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.transitions).toEqual([]);
    });

    it("keeps a negative height as no height too", () => {
        const f = setup();
        expect(f.fs.observed("content", geo(0, 100, -1))).toBe(false);
        expect(f.fs.state()).toEqual(suspended("FOLLOWING"));
    });

    it("asks a following pane to pin on a content or viewport change, leaving the pin to the caller", () => {
        const f = setup();
        expect(f.fs.observed("content", geo(1500))).toBe(true);
        expect(f.fs.observed("viewport", geo(1500))).toBe(true);
        expect(f.host.pin).not.toHaveBeenCalled();
        expect(f.transitions).toEqual([]);
    });

    it("sizes a suspended pane again: a following one asks to pin, a detached one does not", () => {
        const a = setup(suspended("FOLLOWING"));
        expect(a.fs.observed("viewport", geo(0, 800, 500))).toBe(true);
        expect(a.fs.state()).toBe(FOLLOWING);
        expect(a.transitions[0]).toEqual({
            from: suspended("FOLLOWING"),
            to: FOLLOWING,
            cause: "sized",
            detail: "via=viewport ch=500 sh=800 top=0",
        });
        const b = setup(suspended("DETACHED"));
        expect(b.fs.observed("content", geo(0, 800, 500))).toBe(false);
        expect(b.fs.state()).toBe(DETACHED);
    });

    it("follows again, and asks to pin, when a detached pane no longer overflows", () => {
        const f = setup(DETACHED);
        expect(f.fs.observed("content", geo(0, 500, 500))).toBe(true);
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.transitions.map((t) => t.cause)).toEqual(["no-overflow"]);
    });

    it("leaves a detached pane that still overflows alone", () => {
        const f = setup(DETACHED);
        expect(f.fs.observed("content", geo(100, 2000, 500))).toBe(false);
        expect(f.fs.state()).toBe(DETACHED);
        expect(f.transitions).toEqual([]);
    });

    it("tells a following pane's fill pass about the geometry and loads older history while it is short", () => {
        const f = setup();
        f.older.has = true;
        const g = geo(0, 600, 500);
        f.fs.observed("content", g);
        expect(f.host.fillOlder).toHaveBeenCalledTimes(1);
        expect(f.host.fillOlder).toHaveBeenCalledWith(g);
    });

    it("does not fill a pane that is tall enough, has no older history or is loading", () => {
        const f = setup();
        f.older.has = true;
        f.fs.observed("content", geo(0, 1000, 500));
        expect(f.host.fillOlder).not.toHaveBeenCalled();
        f.older.has = false;
        f.fs.observed("content", geo(0, 600, 500));
        expect(f.host.fillOlder).not.toHaveBeenCalled();
        f.older.has = true;
        f.older.loading = true;
        f.fs.observed("content", geo(0, 600, 500));
        expect(f.host.fillOlder).not.toHaveBeenCalled();
    });

    it("does not fill a detached pane", () => {
        const f = setup(DETACHED);
        f.older.has = true;
        f.fs.observed("content", geo(100, 600, 500));
        expect(f.host.fillOlder).not.toHaveBeenCalled();
    });

    it("stops filling after the pass's pages and reports it stalled", () => {
        const f = setup();
        f.older.has = true;
        for (let i = 0; i < FILL_MAX_PAGES; i++) f.fs.checkFill();
        expect(f.host.fillOlder).not.toHaveBeenCalled(); // nothing observed yet: no height
        f.fs.noteGeometry(geo(0, 600, 500));
        for (let i = 0; i < FILL_MAX_PAGES; i++) f.fs.checkFill();
        expect(f.host.fillOlder).toHaveBeenCalledTimes(FILL_MAX_PAGES);
        expect(f.host.fillStalled).not.toHaveBeenCalled();
        f.fs.checkFill();
        expect(f.host.fillOlder).toHaveBeenCalledTimes(FILL_MAX_PAGES);
        expect(f.host.fillStalled).toHaveBeenCalledWith(true);
    });

    it("starts a new fill pass when the viewport grows, or the content is tall enough", () => {
        const f = setup();
        f.older.has = true;
        f.fs.noteGeometry(geo(0, 600, 500));
        for (let i = 0; i <= FILL_MAX_PAGES; i++) f.fs.checkFill();
        expect(f.host.fillStalled).toHaveBeenLastCalledWith(true);
        f.fs.observed("viewport", geo(0, 600, 520));
        expect(f.host.fillStalled).toHaveBeenLastCalledWith(false);
    });

    it("restarts the fill pass on request", () => {
        const f = setup();
        f.older.has = true;
        f.fs.noteGeometry(geo(0, 600, 500));
        for (let i = 0; i <= FILL_MAX_PAGES; i++) f.fs.checkFill();
        f.host.fillOlder.mockClear();
        f.fs.restartFill();
        expect(f.host.fillStalled).toHaveBeenLastCalledWith(false);
        f.fs.checkFill();
        expect(f.host.fillOlder).toHaveBeenCalledTimes(1);
    });

    it("checks for a fill against the geometry it last saw, and does nothing before it has seen any", () => {
        const f = setup();
        f.older.has = true;
        f.fs.checkFill();
        expect(f.host.fillOlder).not.toHaveBeenCalled();
        f.fs.noteGeometry(geo(0, 700, 400));
        f.fs.checkFill();
        expect(f.host.fillOlder).toHaveBeenCalledWith(geo(0, 700, 400));
    });

    it("copes with a host that has no fill or release hooks", () => {
        const f = setup(undefined, false);
        f.older.has = true;
        f.fs.noteGeometry(geo(0, 600, 500));
        for (let i = 0; i <= FILL_MAX_PAGES + 1; i++) f.fs.checkFill();
        mouse("pointerdown", f.child);
        expect(() => mouse("pointerup", window)).not.toThrow();
    });
});

describe("createFollowScroll scrolled", () => {
    it("detaches a following pane on a user scroll up that kept the content's height", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1500));
        f.fs.scrolled(geo(1200), true);
        expect(f.fs.state()).toBe(DETACHED);
        expect(f.transitions).toEqual([
            { from: FOLLOWING, to: DETACHED, cause: "user-scroll", detail: "gap=300px input=gesture" },
        ]);
    });

    it("compares a scroll with the geometry before it, taking the first scroll as having none", () => {
        const f = setup();
        f.fs.scrolled(geo(1200), true);
        expect(f.fs.state()).toBe(FOLLOWING);
        f.fs.scrolled(geo(800), true);
        expect(f.fs.state()).toBe(DETACHED);
    });

    it("does not detach on a scroll that is not the user's, or that goes down, or follows a shrink", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1500));
        f.fs.scrolled(geo(1200), false);
        expect(f.fs.state()).toBe(FOLLOWING);
        f.fs.noteGeometry(geo(1000));
        f.fs.scrolled(geo(1200), true);
        expect(f.fs.state()).toBe(FOLLOWING);
        f.fs.noteGeometry(geo(1500, 2400));
        f.fs.scrolled(geo(1200, 2000), true);
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.transitions).toEqual([]);
    });

    it("labels the input that made a scroll the user's: a held pointer, or the scrollbar", () => {
        const a = setup();
        mouse("pointerdown", a.child);
        a.fs.noteGeometry(geo(1500));
        a.fs.scrolled(geo(1200), true);
        expect(a.transitions[0].detail).toBe("gap=300px input=pointer");
        expect(a.transitions[0].cause).toBe("user-scroll");

        const b = setup();
        mouse("pointerdown", b.scroller);
        b.fs.noteGeometry(geo(1500));
        b.fs.scrolled(geo(1200), true);
        expect(b.transitions[0].cause).toBe("user-scroll:scrollbar");
        expect(b.transitions[0].detail).toBe("gap=300px input=scrollbar");
    });

    it("loads older history from a user scroll near the top while detached, with the geometry just seen", () => {
        const f = setup(DETACHED);
        f.older.has = true;
        const g = geo(30);
        f.fs.noteGeometry(geo(100));
        f.fs.scrolled(g, true);
        expect(f.host.loadOlder).toHaveBeenCalledTimes(1);
        expect(f.host.loadOlder).toHaveBeenCalledWith(g);
    });

    it("loads older history in the same scroll that detaches the pane", () => {
        const f = setup();
        f.older.has = true;
        f.fs.noteGeometry(geo(100));
        f.fs.scrolled(geo(30), true);
        expect(f.fs.state()).toBe(DETACHED);
        expect(f.host.loadOlder).toHaveBeenCalledTimes(1);
    });

    it("does not load older history with none, while loading, away from the top, or for a scroll that is not the user's", () => {
        const f = setup(DETACHED);
        f.fs.noteGeometry(geo(100));
        f.fs.scrolled(geo(30), true);
        f.older.has = true;
        f.older.loading = true;
        f.fs.scrolled(geo(20), true);
        f.older.loading = false;
        f.fs.scrolled(geo(400), true);
        f.fs.scrolled(geo(10), false);
        expect(f.host.loadOlder).not.toHaveBeenCalled();
    });

    it("follows again, and pins through the host, when a user scroll ends within 24 px of the bottom", () => {
        const f = setup(DETACHED);
        f.fs.noteGeometry(geo(900));
        f.fs.scrolled(geo(1480), true);
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.host.pin).toHaveBeenCalledWith("user-scroll-to-bottom");
        expect(f.transitions.map((t) => t.cause)).toEqual(["user-scroll-to-bottom"]);
    });

    it("does not follow again from 25 px above the bottom", () => {
        const f = setup(DETACHED);
        f.fs.noteGeometry(geo(900));
        f.fs.scrolled(geo(1475), true);
        expect(f.fs.state()).toBe(DETACHED);
        expect(f.host.pin).not.toHaveBeenCalled();
    });

    it("ignores a scroll of a suspended pane", () => {
        const f = setup(suspended("FOLLOWING"));
        f.fs.noteGeometry(geo(1500));
        f.fs.scrolled(geo(800), true);
        expect(f.fs.state()).toEqual(suspended("FOLLOWING"));
        expect(f.transitions).toEqual([]);
    });

    it("does not re-attach a pane the reader just left, for a scroll that is only our own write settling", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        wheel(f.scroller, { deltaY: -50 });
        expect(f.fs.state()).toBe(DETACHED);
        f.clock.t += 100;
        f.fs.scrolled(geo(1490), true);
        expect(f.fs.state()).toBe(DETACHED);
        f.clock.t += INTENT_WINDOW_MS;
        f.fs.scrolled(geo(1490), true);
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("counts a scroll 250 ms after an up gesture as its aftermath, and 251 ms as the reader's own", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        wheel(f.scroller, { deltaY: -50 });
        f.clock.t += 250;
        f.fs.scrolled(geo(1490), true);
        expect(f.fs.state()).toBe(DETACHED);
        f.clock.t += 1;
        f.fs.scrolled(geo(1490), true);
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("lets a down gesture's scroll re-attach at once", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        wheel(f.scroller, { deltaY: -50 });
        wheel(f.scroller, { deltaY: 50 });
        f.fs.scrolled(geo(1490), true);
        expect(f.fs.state()).toBe(FOLLOWING);
    });
});

describe("createFollowScroll jumps and reset", () => {
    it("jumps to the bottom: follows again and pins through the host with the reason", () => {
        const f = setup(DETACHED);
        f.fs.jumpToBottom("typing");
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.host.pin).toHaveBeenCalledWith("jump-to-bottom:typing");
        expect(f.transitions.map((t) => t.cause)).toEqual(["jump-to-bottom:typing"]);
    });

    it("pins again on a jump to the bottom even when already following", () => {
        const f = setup();
        f.fs.jumpToBottom("send");
        expect(f.host.pin).toHaveBeenCalledWith("jump-to-bottom:send");
        expect(f.transitions).toEqual([]);
    });

    it("jumps to a node: detaches, naming the first eight characters of the node", () => {
        const f = setup();
        f.fs.jumpToNode("0123456789abcdef");
        expect(f.fs.state()).toBe(DETACHED);
        expect(f.transitions).toEqual([
            { from: FOLLOWING, to: DETACHED, cause: "jump-to-node", detail: "node=01234567" },
        ]);
    });

    it("resets to following", () => {
        const f = setup(DETACHED);
        f.fs.reset();
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.transitions.map((t) => t.cause)).toEqual(["reset"]);
    });

    it("restarts the fill pass on a reset", () => {
        const f = setup();
        f.older.has = true;
        f.fs.noteGeometry(geo(0, 600, 500));
        for (let i = 0; i < 5; i++) f.fs.checkFill();
        f.fs.reset();
        for (let i = 0; i < FILL_MAX_PAGES; i++) f.fs.checkFill();
        expect(f.host.fillOlder).toHaveBeenCalledTimes(5 + FILL_MAX_PAGES);
    });
});

describe("createFollowScroll wheel", () => {
    it("opens the intent window for a wheel, and closes it after 250 ms", () => {
        const f = setup();
        wheel(f.scroller, { deltaY: 40 });
        expect(f.fs.windowOpen()).toBe(true);
        f.clock.t += 250;
        expect(f.fs.windowOpen()).toBe(true);
        f.clock.t += 1;
        expect(f.fs.windowOpen()).toBe(false);
        expect(INTENT_WINDOW_MS).toBe(250);
    });

    it("ignores a zoom wheel (ctrl or meta) and a sideways-only wheel", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        wheel(f.scroller, { deltaY: -40, ctrlKey: true });
        wheel(f.scroller, { deltaY: -40, metaKey: true });
        wheel(f.scroller, { deltaY: 0, deltaX: 40 });
        expect(f.fs.windowOpen()).toBe(false);
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("sees a wheel even when something inside the transcript stops it from bubbling", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        f.child.addEventListener("wheel", (e) => e.stopPropagation());
        wheel(f.child, { deltaY: -40 });
        expect(f.fs.state()).toBe(DETACHED);
    });

    it("detaches a following pane on a wheel up, naming the gap", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        wheel(f.scroller, { deltaY: -40 });
        expect(f.fs.state()).toBe(DETACHED);
        expect(f.transitions).toEqual([
            { from: FOLLOWING, to: DETACHED, cause: "user-escape:wheel-up", detail: "gap=500px" },
        ]);
    });

    it("does not escape on a wheel up at the top, or on a wheel down", () => {
        const f = setup();
        f.fs.noteGeometry(geo(0));
        wheel(f.scroller, { deltaY: -40 });
        expect(f.fs.state()).toBe(FOLLOWING);
        f.fs.noteGeometry(geo(1000));
        wheel(f.scroller, { deltaY: 40 });
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.fs.windowOpen()).toBe(true);
    });

    it("does not escape before any geometry has been seen", () => {
        const f = setup();
        wheel(f.scroller, { deltaY: -40 });
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("defers to a nested scroll-handoff box inside the transcript", () => {
        const f = setup();
        const box = document.createElement("div");
        box.className = "scroll-handoff-box";
        const inner = document.createElement("span");
        box.appendChild(inner);
        f.scroller.appendChild(box);
        f.fs.noteGeometry(geo(1000));
        wheel(inner, { deltaY: -40 });
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.fs.windowOpen()).toBe(true);
    });

    it("escapes on a wheel up in the transcript when the transcript is itself such a box", () => {
        const f = setup();
        f.scroller.classList.add("scroll-handoff-box");
        f.fs.noteGeometry(geo(1000));
        wheel(f.child, { deltaY: -40 });
        expect(f.fs.state()).toBe(DETACHED);
    });

    it("escapes on a wheel up over a box that holds the transcript rather than sits in it", () => {
        const outer = document.createElement("div");
        outer.className = "scroll-handoff-box";
        document.body.appendChild(outer);
        const f = setup();
        outer.appendChild(f.scroller);
        cleanups.push(() => outer.remove());
        f.fs.noteGeometry(geo(1000));
        wheel(f.child, { deltaY: -40 });
        expect(f.fs.state()).toBe(DETACHED);
    });
});

describe("createFollowScroll touch and pointer", () => {
    it("counts a touch start or move as the user's, without escaping", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        f.scroller.dispatchEvent(new Event("touchstart", { bubbles: true }));
        expect(f.fs.windowOpen()).toBe(true);
        f.clock.t += 1000;
        expect(f.fs.windowOpen()).toBe(false);
        f.scroller.dispatchEvent(new Event("touchmove", { bubbles: true }));
        expect(f.fs.windowOpen()).toBe(true);
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("does not count a touch as an up gesture: the scroll that follows it can re-attach", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        wheel(f.scroller, { deltaY: -50 });
        f.scroller.dispatchEvent(new Event("touchstart", { bubbles: true }));
        f.fs.scrolled(geo(1490), true);
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("does not count a scrollbar press as an up gesture either", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        wheel(f.scroller, { deltaY: -50 });
        mouse("pointerdown", f.scroller);
        f.fs.scrolled(geo(1490), true);
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("holds the pointer for a press on the content, without opening the intent window", () => {
        const f = setup();
        mouse("pointerdown", f.child);
        expect(f.fs.pointerHeld()).toBe(true);
        expect(f.fs.scrollbarHeld()).toBe(false);
        expect(f.fs.windowOpen()).toBe(false);
    });

    it("holds the scrollbar for a press on the transcript itself, keeping the window open while held", () => {
        const f = setup();
        mouse("pointerdown", f.scroller);
        expect(f.fs.pointerHeld()).toBe(true);
        expect(f.fs.scrollbarHeld()).toBe(true);
        f.clock.t += 10_000;
        expect(f.fs.windowOpen()).toBe(true);
    });

    it("lets go on pointer up, tells the host, and keeps the window open for a scrollbar drag's trailing scrolls", () => {
        const f = setup();
        mouse("pointerdown", f.scroller);
        f.clock.t += 5000;
        mouse("pointerup", window);
        expect(f.fs.pointerHeld()).toBe(false);
        expect(f.fs.scrollbarHeld()).toBe(false);
        expect(f.host.released).toHaveBeenCalledTimes(1);
        expect(f.fs.windowOpen()).toBe(true);
        f.clock.t += INTENT_WINDOW_MS + 1;
        expect(f.fs.windowOpen()).toBe(false);
    });

    it("does not open the window or tell the host for a pointer up that follows no press", () => {
        const f = setup();
        mouse("pointerup", window);
        expect(f.host.released).not.toHaveBeenCalled();
        expect(f.fs.windowOpen()).toBe(false);
    });

    it("tells the host once for a content press released", () => {
        const f = setup();
        mouse("pointerdown", f.child);
        mouse("pointerup", window);
        mouse("pointerup", window);
        expect(f.host.released).toHaveBeenCalledTimes(1);
        expect(f.fs.windowOpen()).toBe(false);
    });

    it("lets go when the press is cancelled", () => {
        const f = setup();
        mouse("pointerdown", f.child);
        mouse("pointercancel", window);
        expect(f.fs.pointerHeld()).toBe(false);
        expect(f.host.released).toHaveBeenCalledTimes(1);
    });

    it("lets go when the pointer moves with no button down, and not while one is", () => {
        const f = setup();
        mouse("pointerdown", f.child);
        mouse("pointermove", document, { buttons: 1 });
        expect(f.fs.pointerHeld()).toBe(true);
        mouse("pointermove", document, { buttons: 0 });
        expect(f.fs.pointerHeld()).toBe(false);
        expect(f.host.released).toHaveBeenCalledTimes(1);
    });

    it("ignores a pointer move when nothing is held", () => {
        const f = setup();
        mouse("pointermove", document, { buttons: 0 });
        expect(f.host.released).not.toHaveBeenCalled();
    });

    it("lets go when the window loses focus, but not when an element does", () => {
        const f = setup();
        mouse("pointerdown", f.child);
        const el = document.createElement("input");
        document.body.appendChild(el);
        cleanups.push(() => el.remove());
        el.dispatchEvent(new Event("blur"));
        expect(f.fs.pointerHeld()).toBe(true);
        window.dispatchEvent(new Event("blur"));
        expect(f.fs.pointerHeld()).toBe(false);
    });

    it("lets go when the page is hidden, but not when it is visible", () => {
        const f = setup();
        mouse("pointerdown", f.child);
        const state = vi.spyOn(document, "visibilityState", "get");
        state.mockReturnValue("visible");
        document.dispatchEvent(new Event("visibilitychange"));
        expect(f.fs.pointerHeld()).toBe(true);
        state.mockReturnValue("hidden");
        document.dispatchEvent(new Event("visibilitychange"));
        expect(f.fs.pointerHeld()).toBe(false);
    });
});

describe("createFollowScroll keys", () => {
    function focusIn(f: Fixture): HTMLElement {
        const btn = document.createElement("button");
        f.scroller.appendChild(btn);
        return btn;
    }

    it("escapes on an up key aimed at the transcript, and counts it as an up gesture", () => {
        for (const k of ["PageUp", "Home", "ArrowUp"]) {
            const f = setup();
            f.fs.noteGeometry(geo(1000));
            key(focusIn(f), k);
            expect(f.fs.state()).toBe(DETACHED);
            expect(f.transitions[0].cause).toBe("user-escape:key");
            expect(f.fs.windowOpen()).toBe(true);
        }
    });

    it("escapes on shift+space, but not on space alone", () => {
        const a = setup();
        a.fs.noteGeometry(geo(1000));
        key(focusIn(a), " ", { shiftKey: true });
        expect(a.fs.state()).toBe(DETACHED);
        const b = setup();
        b.fs.noteGeometry(geo(1000));
        key(focusIn(b), " ");
        expect(b.fs.state()).toBe(FOLLOWING);
        expect(b.fs.windowOpen()).toBe(true);
    });

    it("opens the window without escaping for the down keys", () => {
        for (const k of ["PageDown", "End", "ArrowDown"]) {
            const f = setup();
            f.fs.noteGeometry(geo(1000));
            key(focusIn(f), k);
            expect(f.fs.state()).toBe(FOLLOWING);
            expect(f.fs.windowOpen()).toBe(true);
        }
    });

    it("ignores keys that do not scroll", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        key(focusIn(f), "a");
        key(focusIn(f), "Enter");
        expect(f.fs.windowOpen()).toBe(false);
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("ignores keys typed into a text field, a select or an editable element", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        for (const tag of ["input", "textarea", "select"]) {
            const el = document.createElement(tag);
            f.scroller.appendChild(el);
            key(el, "ArrowUp");
        }
        const editable = document.createElement("div");
        Object.defineProperty(editable, "isContentEditable", { value: true });
        f.scroller.appendChild(editable);
        key(editable, "ArrowUp");
        expect(f.fs.windowOpen()).toBe(false);
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("ignores keys aimed at something outside the transcript", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        const other = document.createElement("button");
        document.body.appendChild(other);
        cleanups.push(() => other.remove());
        key(other, "ArrowUp");
        expect(f.fs.windowOpen()).toBe(false);
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("ignores keys with nothing focused before any press has been seen", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        key(document.body, "ArrowUp");
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.fs.windowOpen()).toBe(false);
    });

    it("ignores keys aimed at an element outside the transcript even when the last press was on it", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        mouse("pointerdown", f.child);
        const other = document.createElement("button");
        document.body.appendChild(other);
        cleanups.push(() => other.remove());
        key(other, "ArrowUp");
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("counts an up key as an up gesture: the scroll it causes does not re-attach", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        key(focusIn(f), "PageUp");
        expect(f.fs.state()).toBe(DETACHED);
        f.fs.scrolled(geo(1490), true);
        expect(f.fs.state()).toBe(DETACHED);
    });

    it("takes keys with nothing focused when the last press was on the transcript", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        mouse("pointerdown", f.child);
        mouse("pointerup", window);
        key(document.body, "ArrowUp");
        expect(f.fs.state()).toBe(DETACHED);
    });

    it("takes keys aimed at the page itself as nothing focused", () => {
        const a = setup();
        a.fs.noteGeometry(geo(1000));
        mouse("pointerdown", a.child);
        key(document, "PageUp");
        expect(a.fs.state()).toBe(DETACHED);
        const b = setup();
        b.fs.noteGeometry(geo(1000));
        mouse("pointerdown", b.child);
        key(document.documentElement, "PageUp");
        expect(b.fs.state()).toBe(DETACHED);
    });

    it("ignores keys with nothing focused when the last press was elsewhere", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        const other = document.createElement("div");
        document.body.appendChild(other);
        cleanups.push(() => other.remove());
        mouse("pointerdown", other);
        key(document.body, "ArrowUp");
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.fs.windowOpen()).toBe(false);
    });

    it("forgets that the last press was on the transcript once a press lands elsewhere", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        const other = document.createElement("div");
        document.body.appendChild(other);
        cleanups.push(() => other.remove());
        mouse("pointerdown", f.child);
        mouse("pointerdown", other);
        key(document.body, "ArrowUp");
        expect(f.fs.state()).toBe(FOLLOWING);
    });
});

describe("createFollowScroll selection", () => {
    function select(node: Node): void {
        const range = document.createRange();
        range.selectNodeContents(node);
        const sel = document.getSelection() as Selection;
        sel.removeAllRanges();
        sel.addRange(range);
    }

    it("escapes while a pointer is held and the selection started in the transcript", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        mouse("pointerdown", f.child);
        select(f.child);
        document.dispatchEvent(new Event("selectionchange"));
        expect(f.fs.state()).toBe(DETACHED);
        expect(f.transitions[0].cause).toBe("user-escape:selection");
    });

    it("does nothing without a held pointer", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        select(f.child);
        document.dispatchEvent(new Event("selectionchange"));
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("does nothing for a collapsed selection, one outside the transcript, or none", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        mouse("pointerdown", f.child);
        const sel = document.getSelection() as Selection;
        sel.removeAllRanges();
        document.dispatchEvent(new Event("selectionchange"));
        const collapsed = document.createRange();
        collapsed.setStart(f.child.firstChild as Node, 2);
        collapsed.collapse(true);
        sel.addRange(collapsed);
        document.dispatchEvent(new Event("selectionchange"));
        const outside = document.createElement("p");
        outside.textContent = "elsewhere";
        document.body.appendChild(outside);
        cleanups.push(() => outside.remove());
        select(outside);
        document.dispatchEvent(new Event("selectionchange"));
        expect(f.fs.state()).toBe(FOLLOWING);
    });

    it("does nothing for a pane that is not following", () => {
        const f = setup(DETACHED);
        f.fs.noteGeometry(geo(1000));
        mouse("pointerdown", f.child);
        select(f.child);
        document.dispatchEvent(new Event("selectionchange"));
        expect(f.transitions).toEqual([]);
    });
});

describe("createFollowScroll dispose", () => {
    it("removes every listener", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        f.fs.dispose();
        wheel(f.scroller, { deltaY: -40 });
        f.scroller.dispatchEvent(new Event("touchstart", { bubbles: true }));
        mouse("pointerdown", f.scroller);
        key(f.child, "ArrowUp");
        mouse("pointerdown", f.child);
        expect(f.fs.state()).toBe(FOLLOWING);
        expect(f.fs.windowOpen()).toBe(false);
        expect(f.fs.pointerHeld()).toBe(false);
        expect(f.fs.scrollbarHeld()).toBe(false);
    });

    it("removes the touch move listener", () => {
        const f = setup();
        f.fs.dispose();
        f.scroller.dispatchEvent(new Event("touchmove", { bubbles: true }));
        expect(f.fs.windowOpen()).toBe(false);
    });

    it.each([
        ["pointer up", () => mouse("pointerup", window)],
        ["pointer cancel", () => mouse("pointercancel", window)],
        ["window blur", () => window.dispatchEvent(new Event("blur"))],
        ["pointer move", () => mouse("pointermove", document, { buttons: 0 })],
        [
            "page hidden",
            () => {
                vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
                document.dispatchEvent(new Event("visibilitychange"));
            },
        ],
    ])("stops letting go of a held pointer on %s", (_name, fire) => {
        const f = setup();
        mouse("pointerdown", f.child);
        f.fs.dispose();
        fire();
        expect(f.host.released).not.toHaveBeenCalled();
        expect(f.fs.pointerHeld()).toBe(true);
    });

    it("stops reading the selection", () => {
        const f = setup();
        f.fs.noteGeometry(geo(1000));
        mouse("pointerdown", f.child);
        f.fs.dispose();
        const range = document.createRange();
        range.selectNodeContents(f.child);
        const sel = document.getSelection() as Selection;
        sel.removeAllRanges();
        sel.addRange(range);
        document.dispatchEvent(new Event("selectionchange"));
        expect(f.fs.state()).toBe(FOLLOWING);
    });
});
