// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The follow state machine wired into the transcript (`followController`):
 * SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md §8 Phase 2. Same
 * fake ResizeObserver / rAF / geometry harness as
 * AgentDocumentVirtualList.pin.test.tsx, run on both pin paths (the content
 * ResizeObserver, and the one-way flow).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DocumentNode, DocumentState } from "../types";
import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";

type ROCallback = (entries: ResizeObserverEntry[]) => void;
let roInstances: { callback: ROCallback; targets: Set<Element> }[] = [];
class FakeResizeObserver {
    private targets = new Set<Element>();
    constructor(private callback: ROCallback) {
        roInstances.push({ callback: this.callback, targets: this.targets });
    }
    observe(el: Element): void {
        this.targets.add(el);
    }
    unobserve(el: Element): void {
        this.targets.delete(el);
    }
    disconnect(): void {
        this.targets.clear();
    }
}
/** One ResizeObserver round: every observer watching `el`, in creation order. */
function triggerResize(el: Element): void {
    for (const { callback, targets } of roInstances)
        if (targets.has(el)) callback([{ target: el } as ResizeObserverEntry]);
}

let rafQueue: FrameRequestCallback[] = [];
function flushRaf(): void {
    for (let i = 0; i < 200 && rafQueue.length > 0; i++) {
        const pending = rafQueue;
        rafQueue = [];
        for (const cb of pending) cb(i * 16);
    }
}

interface Geo {
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
}

/**
 * Scroller geometry: writes clamp, and a write that moves fires `scroll` like a
 * browser. `scrollHeight` is the content's plus the one-way bottom spacer's.
 */
function makeScrollable(el: HTMLElement, geo: Geo) {
    let { scrollTop, clientHeight } = geo;
    let content = geo.scrollHeight;
    const spacer = (): number =>
        parseFloat(el.querySelector<HTMLElement>(".agent-document-bottom-spacer")?.style.height ?? "") || 0;
    const sh = (): number => content + spacer();
    const clamp = (v: number): number => Math.max(0, Math.min(v, Math.max(0, sh() - clientHeight)));
    const write = (v: number): void => {
        const next = clamp(v);
        if (next === scrollTop) return;
        scrollTop = next;
        el.dispatchEvent(new Event("scroll"));
    };
    Object.defineProperty(el, "scrollTop", { configurable: true, get: () => scrollTop, set: write });
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: sh });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => clientHeight });
    el.scrollTo = ((opts?: ScrollToOptions | number) =>
        write(typeof opts === "number" ? opts : (opts?.top ?? scrollTop))) as typeof el.scrollTo;
    return {
        set(g: Partial<Geo>) {
            if (g.scrollHeight !== undefined) content = g.scrollHeight;
            if (g.clientHeight !== undefined) clientHeight = g.clientHeight;
            if (g.scrollTop !== undefined) scrollTop = g.scrollTop;
        },
        get top() {
            return scrollTop;
        },
        get content() {
            return content;
        },
    };
}

const emptyDocumentState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    heldOpenNodes: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

const md = (id: string): DocumentNode => ({ type: "markdown", id, content: `text ${id}`, timestamp: 0 });

/** At `px`, or within the 1 px the one-way follower comes to rest at (follower.ts REST_PX). */
function expectAt(top: number, px: number): void {
    expect(top).toBeGreaterThan(px - 1);
    expect(top).toBeLessThanOrEqual(px);
}

let clock = 0;
let info: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
    roInstances = [];
    rafQueue = [];
    clock = 0;
    vi.spyOn(performance, "now").mockImplementation(() => clock);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        rafQueue.push(cb);
        return rafQueue.length;
    });
    vi.stubGlobal("cancelAnimationFrame", () => {});
    info = vi.spyOn(console, "info").mockImplementation(() => {});
});

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.getSelection()?.removeAllRanges();
});

/** The `[scroll-follow]` lines logged so far, each joined into one string. */
const followLines = (): string[] =>
    info.mock.calls.filter((c) => c[0] === "[scroll-follow]").map((c) => c.slice(1).join(" "));

function setup(opts: { geo: Geo; oneWay?: boolean; hasOlder?: boolean }) {
    const [nodes, setNodes] = createSignal<DocumentNode[]>([md("a")]);
    const viewState = createAgentViewState(nodes);
    const [docState] = createSignal(emptyDocumentState());
    const onLoadOlder = vi.fn(() => new Promise<void>(() => {})); // never resolves: one page in flight
    let jump: ((reason?: string) => void) | undefined;
    const utils = render(() => (
        <AgentDocumentVirtualList
            viewState={viewState}
            documentState={docState}
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
            blockId="abcdef1234567890"
            onLoadOlder={onLoadOlder}
            hasOlderHistory={() => opts.hasOlder ?? false}
            followController
            oneWayFlow={opts.oneWay ?? false}
            scrollToBottomRef={(fn) => {
                jump = fn;
            }}
        />
    ));
    const scrollRef = utils.container.querySelector(".agent-document") as HTMLElement;
    const buffer = utils.container.querySelector(".agent-document-streaming-buffer") as HTMLElement;
    const row = document.createElement("div");
    row.textContent = "some transcript text to select";
    buffer.appendChild(row);
    const g = makeScrollable(scrollRef, opts.geo);
    triggerResize(buffer); // first observation: pins
    flushRaf();
    return {
        viewState,
        scrollRef,
        buffer,
        row,
        g,
        onLoadOlder,
        setNodes,
        jump: (r?: string) => jump!(r),
        /** Content grows: the browser lays it out and runs the ResizeObservers. */
        grow(px: number) {
            g.set({ scrollHeight: g.content + px });
            triggerResize(buffer);
            flushRaf();
        },
        wheel(deltaY: number, target: Element = scrollRef) {
            target.dispatchEvent(new WheelEvent("wheel", { deltaY, bubbles: true }));
        },
        /** The browser scrolls (the gesture's own scroll), then the batch runs. */
        scrollTo(top: number) {
            g.set({ scrollTop: top });
            scrollRef.dispatchEvent(new Event("scroll"));
            flushRaf();
        },
        following: () => viewState.stickToBottom(),
    };
}

for (const oneWay of [false, true]) {
    describe(`follow controller (${oneWay ? "one-way flow" : "content ResizeObserver pin"})`, () => {
        const geo = { scrollTop: 0, scrollHeight: 1000, clientHeight: 300 };

        it("marks the scroller and follows from the first observation", () => {
            const s = setup({ geo, oneWay });
            expect(s.scrollRef.dataset.followController).toBe("true");
            expect(s.following()).toBe(true);
            expect(s.g.top).toBe(700);
            s.grow(200);
            expectAt(s.g.top, 900);
        });

        it("a wheel up detaches at once, before any scroll batch, and content then streams in below without moving it", () => {
            const s = setup({ geo, oneWay });
            s.wheel(-40);
            expect(s.following()).toBe(false); // synchronous: a same-frame pin can't override it
            s.scrollTo(660);
            clock += 1_000;
            s.grow(500);
            s.grow(500);
            expect(s.g.top).toBe(660);
            expect(s.following()).toBe(false);
        });

        it("scrolling back to the bottom follows again; stopping 30 px short doesn't", () => {
            const s = setup({ geo, oneWay });
            s.wheel(-100);
            s.scrollTo(600);
            clock += 1_000;
            s.wheel(70);
            s.scrollTo(670); // 30 px above the bottom
            expect(s.following()).toBe(false);
            clock += 1_000;
            s.wheel(20);
            s.scrollTo(690); // within REATTACH_PX
            expect(s.following()).toBe(true);
            expect(s.g.top).toBe(700); // and pinned
            clock += 1_000;
            s.grow(300);
            expect(s.g.top).toBe(1000);
        });

        it("a wheel down never detaches", () => {
            const s = setup({ geo, oneWay });
            s.wheel(100);
            s.scrollTo(700);
            expect(s.following()).toBe(true);
        });

        it("a wheel up in a pane that doesn't overflow yet doesn't detach (it scrolls nothing)", () => {
            const s = setup({ geo: { scrollTop: 0, scrollHeight: 200, clientHeight: 300 }, oneWay });
            s.wheel(-100);
            expect(s.following()).toBe(true);
            clock += 1_000;
            s.grow(400); // the first overflow follows
            expectAt(s.g.top, 300);
        });

        it("a wheel up over a nested preview box waits for the scroll it relays", () => {
            const s = setup({ geo, oneWay });
            const box = document.createElement("div");
            box.className = "scroll-handoff-box";
            s.row.appendChild(box);
            s.wheel(-40, box);
            expect(s.following()).toBe(true); // the box may have used it
            s.scrollTo(660); // it relayed: the transcript moved up
            expect(s.following()).toBe(false);
        });

        it("A: a new pane's first small overflow pins near the top and neither pages nor detaches", () => {
            const s = setup({ geo: { scrollTop: 0, scrollHeight: 312, clientHeight: 300 }, oneWay, hasOlder: true });
            expectAt(s.g.top, 12);
            expect(s.onLoadOlder).not.toHaveBeenCalled();
            expect(s.following()).toBe(true);
            s.grow(400);
            expectAt(s.g.top, 412);
        });

        it("B1: a click on content is not a gesture; a pin minutes later with a big same-frame flush keeps following", () => {
            const s = setup({ geo, oneWay });
            s.row.dispatchEvent(new Event("pointerdown", { bubbles: true }));
            window.dispatchEvent(new Event("pointerup"));
            clock += 60_000;
            s.g.set({ scrollHeight: 1400 });
            triggerResize(s.buffer);
            s.g.set({ scrollHeight: 2000 }); // a big tool output lands before the batch
            flushRaf();
            expect(s.following()).toBe(true);
        });

        it("B2: a scroll the browser makes on its own never detaches; the next resize pins back", () => {
            const s = setup({ geo, oneWay });
            s.scrollTo(150); // anchoring / clamp, no gesture
            expect(s.following()).toBe(true);
            s.grow(100);
            expect(s.g.top).toBe(800);
        });

        it("pages only from a user scroll near the top, once detached", () => {
            const s = setup({ geo, oneWay, hasOlder: true });
            s.scrollTo(10); // browser-made: nothing
            expect(s.onLoadOlder).not.toHaveBeenCalled();
            s.grow(0);
            s.wheel(-700);
            s.scrollTo(10);
            expect(s.following()).toBe(false);
            expect(s.onLoadOlder).toHaveBeenCalledTimes(1);
            clock += 1_000;
            s.wheel(-5);
            s.scrollTo(5); // the page is still loading: no second request
            expect(s.onLoadOlder).toHaveBeenCalledTimes(1);
        });

        it("never pages without older history", () => {
            const s = setup({ geo, oneWay, hasOlder: false });
            s.wheel(-700);
            s.scrollTo(0);
            expect(s.onLoadOlder).not.toHaveBeenCalled();
        });

        it("a scrollbar drag up detaches, as the scrollbar's", () => {
            const s = setup({ geo, oneWay });
            s.scrollRef.dispatchEvent(new Event("pointerdown")); // on the scroller itself: its scrollbar
            clock += 2_000; // a slow drag: the held pointer keeps it the user's
            s.scrollTo(400);
            window.dispatchEvent(new Event("pointerup"));
            expect(s.following()).toBe(false);
            expect(followLines()).toContain("pane=abcdef1 following→detached cause=user-scroll:scrollbar gap=300px");
        });

        it("a text-selection drag in the transcript detaches", () => {
            const s = setup({ geo, oneWay });
            s.row.dispatchEvent(new Event("pointerdown", { bubbles: true }));
            const range = document.createRange();
            range.setStart(s.row.firstChild!, 0);
            range.setEnd(s.row.firstChild!, 4);
            document.getSelection()!.addRange(range);
            document.dispatchEvent(new Event("selectionchange"));
            window.dispatchEvent(new Event("pointerup"));
            expect(s.following()).toBe(false);
            expect(followLines()).toContain("pane=abcdef1 following→detached cause=user-escape:selection gap=0px");
        });

        it("scroll keys count only for this transcript", () => {
            const s = setup({ geo, oneWay });
            const ta = document.createElement("textarea");
            document.body.appendChild(ta);
            ta.dispatchEvent(new KeyboardEvent("keydown", { key: "PageUp", bubbles: true }));
            expect(s.following()).toBe(true); // typing in an editor
            const elsewhere = document.createElement("div");
            document.body.appendChild(elsewhere);
            elsewhere.dispatchEvent(new Event("pointerdown", { bubbles: true }));
            document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "PageUp", bubbles: true }));
            expect(s.following()).toBe(true); // the last press was in another pane
            s.row.dispatchEvent(new Event("pointerdown", { bubbles: true }));
            window.dispatchEvent(new Event("pointerup"));
            document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
            expect(s.following()).toBe(true); // down keys never escape
            document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "PageUp", bubbles: true }));
            expect(s.following()).toBe(false);
            ta.remove();
            elsewhere.remove();
        });

        it("a jump to bottom (send) follows again and pins, logging its reason", () => {
            const s = setup({ geo, oneWay });
            s.wheel(-300);
            s.scrollTo(400);
            clock += 1_000;
            s.g.set({ scrollHeight: 1500 });
            s.jump("sent");
            expect(s.following()).toBe(true);
            expect(s.g.top).toBe(1200);
            expect(followLines().slice(-1)).toEqual(["pane=abcdef1 detached→following cause=jump-to-bottom:sent"]);
        });

        it("hidden suspends and shown restores, pinning a following pane", () => {
            const s = setup({ geo, oneWay });
            s.g.set({ clientHeight: 0 });
            triggerResize(s.scrollRef);
            s.scrollTo(0); // a hidden pane's scrollTop reset
            s.g.set({ clientHeight: 300, scrollHeight: 1200 });
            triggerResize(s.scrollRef);
            flushRaf();
            expect(s.following()).toBe(true);
            expect(s.g.top).toBe(900);
            expect(followLines()).toEqual(
                expect.arrayContaining([
                    "pane=abcdef1 following→suspended cause=zero-size",
                    "pane=abcdef1 suspended→following cause=sized",
                ])
            );
        });

        it("/clear follows the new conversation", () => {
            const s = setup({ geo, oneWay });
            s.wheel(-300);
            s.scrollTo(400);
            expect(s.following()).toBe(false);
            s.setNodes([]);
            expect(s.following()).toBe(true);
            expect(followLines().slice(-1)).toEqual(["pane=abcdef1 detached→following cause=reset"]);
        });

        it("every transition logs exactly one line with its cause (I4)", () => {
            const s = setup({ geo, oneWay });
            s.wheel(-200);
            s.scrollTo(500);
            clock += 1_000;
            s.wheel(200);
            s.scrollTo(700);
            expect(followLines()).toEqual([
                "pane=abcdef1 mount following",
                "pane=abcdef1 following→detached cause=user-escape:wheel-up gap=0px",
                "pane=abcdef1 detached→following cause=user-scroll-to-bottom gap=0px",
            ]);
            expect(s.scrollRef.dataset.followState).toBe("following");
        });
    });
}
