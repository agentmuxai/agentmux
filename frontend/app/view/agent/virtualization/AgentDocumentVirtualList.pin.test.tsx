// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pin-to-bottom without forced synchronous layout — Phase 1 of
 * docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.1.
 *
 * jsdom has no layout, so "no forced layout" is asserted structurally: the
 * scroller's geometry getters are counted, and a path that must not force
 * layout must not read them at all. Reads inside a ResizeObserver callback are
 * allowed (layout is already clean there in a real browser); reads from a
 * Solid effect or from the scroll handler of our own pin are not.
 *
 * Same fake ResizeObserver / rAF / geometry approach as
 * AgentDocumentVirtualList.resize.test.tsx.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";
import type { DocumentNode, DocumentState } from "../types";

afterEach(() => cleanup());

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
function triggerResize(el: Element): void {
    for (const { callback, targets } of roInstances) if (targets.has(el)) callback([{ target: el } as ResizeObserverEntry]);
}

let rafQueue: FrameRequestCallback[] = [];
function flushRaf(): void {
    const pending = rafQueue;
    rafQueue = [];
    for (const cb of pending) cb(0);
}

interface Geo {
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
}

/** Real-ish scroller geometry with a read counter. scrollTo() clamps and,
 *  like a browser, fires `scroll` only when the position actually changes. */
function makeScrollable(el: HTMLElement, geo: Geo) {
    let { scrollTop, scrollHeight, clientHeight } = geo;
    const reads = { count: 0 };
    Object.defineProperty(el, "scrollTop", {
        configurable: true,
        get: () => (reads.count++, scrollTop),
        set: (v: number) => {
            scrollTop = v;
        },
    });
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => (reads.count++, scrollHeight) });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => (reads.count++, clientHeight) });
    el.scrollTo = ((opts?: ScrollToOptions | number) => {
        const requested = typeof opts === "number" ? opts : (opts?.top ?? scrollTop);
        const next = Math.max(0, Math.min(requested, Math.max(0, scrollHeight - clientHeight)));
        if (next !== scrollTop) {
            scrollTop = next;
            el.dispatchEvent(new Event("scroll"));
        }
    }) as typeof el.scrollTo;
    return {
        reads,
        set(g: Partial<Geo>) {
            if (g.scrollHeight !== undefined) scrollHeight = g.scrollHeight;
            if (g.clientHeight !== undefined) clientHeight = g.clientHeight;
            if (g.scrollTop !== undefined) scrollTop = g.scrollTop;
        },
        get top() {
            return scrollTop;
        },
    };
}

const emptyDocumentState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    expandedTools: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

const md = (id: string): DocumentNode => ({ type: "markdown", id, content: `text ${id}`, timestamp: 0 });

function setup() {
    const [nodes, setNodes] = createSignal<DocumentNode[]>([md("a")]);
    const viewState = createAgentViewState(nodes);
    const [docState] = createSignal(emptyDocumentState());
    const utils = render(() => (
        <AgentDocumentVirtualList viewState={viewState} documentState={docState} onToggleCollapse={() => {}} onTogglePin={() => {}} />
    ));
    const scrollRef = utils.container.querySelector(".agent-document") as HTMLElement;
    const buffer = utils.container.querySelector(".agent-document-streaming-buffer") as HTMLElement;
    const g = makeScrollable(scrollRef, { scrollTop: 0, scrollHeight: 500, clientHeight: 300 });
    return { viewState, scrollRef, buffer, g, setNodes };
}

/** Content grows by `px`: the buffer resizes, the browser runs the RO. */
function grow(s: ReturnType<typeof setup>, px: number) {
    s.g.set({ scrollHeight: s.scrollRef.scrollHeight + px });
    triggerResize(s.buffer);
}

describe("pin-to-bottom without forced layout (Phase 1)", () => {
    beforeEach(() => {
        roInstances = [];
        rafQueue = [];
        vi.stubGlobal("ResizeObserver", FakeResizeObserver);
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            rafQueue.push(cb);
            return rafQueue.length;
        });
        vi.stubGlobal("cancelAnimationFrame", () => {});
        vi.spyOn(console, "info").mockImplementation(() => {});
    });

    it("a stream flush reads no scroller geometry (the pin waits for the ResizeObserver)", async () => {
        const s = setup();
        s.g.reads.count = 0;
        s.setNodes((n) => [...n, md("b"), md("c")]);
        await Promise.resolve(); // any microtask the old effect would have queued
        expect(s.g.reads.count).toBe(0);
        expect(s.g.top).toBe(0);
    });

    it("the content ResizeObserver pins to true bottom and updates overflow state", () => {
        const s = setup();
        grow(s, 0); // first observation: 500 over 300
        expect(s.g.top).toBe(200);
        expect(s.viewState.isOverflowing()).toBe(true);
        grow(s, 120);
        expect(s.g.top).toBe(320);
    });

    it("the scroll event our own pin causes is handled with zero geometry reads", () => {
        const s = setup();
        grow(s, 100);
        s.g.reads.count = 0;
        flushRaf(); // handleScrollNow for the pin's scroll event
        expect(s.g.reads.count).toBe(0);
        expect(s.viewState.stickToBottom()).toBe(true);
    });

    it("a pin that does not move leaves nothing for a later scroll event to trust", () => {
        const s = setup();
        grow(s, 0);
        flushRaf();
        grow(s, 0); // already at true bottom: no scroll event, nothing pending
        // Content shrinks; the browser clamps scrollTop and fires scroll on its own.
        s.g.set({ scrollHeight: 400, scrollTop: 100 });
        s.scrollRef.dispatchEvent(new Event("scroll"));
        s.g.reads.count = 0;
        flushRaf();
        expect(s.g.reads.count).toBeGreaterThan(0); // live geometry, not a stale pin
        expect(s.viewState.stickToBottom()).toBe(true);
    });

    it("user scroll input in the same batch as a pin reads live geometry and disengages", () => {
        const s = setup();
        grow(s, 100); // pin scrolls to 300; its scroll event is pending
        s.scrollRef.dispatchEvent(new Event("wheel"));
        s.g.set({ scrollTop: 40 }); // the user scrolled far up in the same frame
        s.scrollRef.dispatchEvent(new Event("scroll"));
        s.g.reads.count = 0;
        flushRaf();
        expect(s.g.reads.count).toBeGreaterThan(0);
        expect(s.viewState.stickToBottom()).toBe(false);
    });

    it("without user input, 'not near bottom' after our pin keeps following (content landed in the same tick)", () => {
        const s = setup();
        grow(s, 100);
        // More content arrived before the scroll batch ran; no user input.
        s.g.set({ scrollHeight: 900 });
        flushRaf();
        expect(s.viewState.stickToBottom()).toBe(true);
    });

    it("once disengaged, content growth does not pull the reader back", () => {
        const s = setup();
        grow(s, 100);
        flushRaf();
        s.scrollRef.dispatchEvent(new Event("pointerdown"));
        s.g.set({ scrollTop: 10 });
        s.scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();
        expect(s.viewState.stickToBottom()).toBe(false);
        grow(s, 500);
        expect(s.g.top).toBe(10);
    });

    it("a viewport shrink while pinned releases held-open tools that went off the top (clientHeight RO pin)", () => {
        // ReAgent P1 on #3599: this pin source must run the collapse itself —
        // its scroll event is a trusted pin batch, which skips it.
        const [nodes] = createSignal<DocumentNode[]>([md("a")]);
        const viewState = createAgentViewState(nodes);
        const held = { ...emptyDocumentState(), expandedTools: new Set(["tool-off-screen"]) };
        const [docState] = createSignal(held);
        const release = vi.fn();
        const utils = render(() => (
            <AgentDocumentVirtualList
                viewState={viewState}
                documentState={docState}
                onToggleCollapse={() => {}}
                onTogglePin={() => {}}
                onReleaseToolOpen={release}
            />
        ));
        const scrollRef = utils.container.querySelector(".agent-document") as HTMLElement;
        const g = makeScrollable(scrollRef, { scrollTop: 200, scrollHeight: 500, clientHeight: 300 });
        // A laid-out container (a zero-size rect means "hidden pane": skipped).
        scrollRef.getBoundingClientRect = () => ({ x: 0, y: 0, top: 0, left: 0, bottom: 300, right: 400, width: 400, height: 300, toJSON: () => ({}) }) as DOMRect;
        // The composer grew: the container's own viewport shrinks by 100 px.
        g.set({ clientHeight: 200 });
        triggerResize(scrollRef); // only the clientHeight RO observes scrollRef
        flushRaf(); // the pin's own scroll event: a trusted batch
        expect(g.top).toBe(300);
        // The held-open tool is not rendered any more (scrolled away): released.
        expect(release).toHaveBeenCalledWith("tool-off-screen");
    });

    it("typing in an editable element is not scroll input; PageUp elsewhere is", () => {
        const s = setup();
        const ta = document.createElement("textarea");
        document.body.appendChild(ta);
        grow(s, 100);
        ta.dispatchEvent(new KeyboardEvent("keydown", { key: "PageUp", bubbles: true }));
        s.g.reads.count = 0;
        flushRaf();
        expect(s.g.reads.count).toBe(0); // still a trusted pin batch
        grow(s, 100);
        document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "PageUp", bubbles: true }));
        s.g.reads.count = 0;
        flushRaf();
        expect(s.g.reads.count).toBeGreaterThan(0);
        ta.remove();
    });
});
