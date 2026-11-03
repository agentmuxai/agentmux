// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The scroll handler's own contract, pinned before the component is split
 * into named hooks: one handler run per animation frame, no layout reads in
 * the batch our own pin causes (and live reads when the user may have caused
 * it), the "accepted" jump that never pulls a reader down, and every observer
 * released on unmount. Same fake ResizeObserver / rAF / geometry harness as
 * AgentDocumentVirtualList.follow-controller.test.tsx.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DocumentNode, DocumentState } from "../../../agent-model/types";
import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";

type ROCallback = (entries: ResizeObserverEntry[]) => void;
interface FakeRO {
    callback: ROCallback;
    targets: Set<Element>;
    disconnected: boolean;
}
let roInstances: FakeRO[] = [];
class FakeResizeObserver {
    private record: FakeRO;
    constructor(callback: ROCallback) {
        this.record = { callback, targets: new Set<Element>(), disconnected: false };
        roInstances.push(this.record);
    }
    observe(el: Element): void {
        this.record.targets.add(el);
    }
    unobserve(el: Element): void {
        this.record.targets.delete(el);
    }
    disconnect(): void {
        this.record.targets.clear();
        this.record.disconnected = true;
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

/** Layout reads made from inside the scroll handler (`handleScrollNow`), told apart by the call stack. */
const reads = { inHandler: 0 };
const fromHandler = (): boolean => new Error().stack?.includes("handleScrollNow") ?? false;

/** Writes clamp, and a write that moves fires `scroll` like a browser. */
function makeScrollable(el: HTMLElement, geo: Geo) {
    let { scrollTop, clientHeight } = geo;
    let content = geo.scrollHeight;
    const clamp = (v: number): number => Math.max(0, Math.min(v, Math.max(0, content - clientHeight)));
    const write = (v: number): void => {
        const next = clamp(v);
        if (next === scrollTop) return;
        scrollTop = next;
        el.dispatchEvent(new Event("scroll"));
    };
    const counted = <T,>(v: () => T) => () => {
        if (fromHandler()) reads.inHandler++;
        return v();
    };
    Object.defineProperty(el, "scrollTop", { configurable: true, get: counted(() => scrollTop), set: write });
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: counted(() => content) });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: counted(() => clientHeight) });
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

let clock = 0;
const cancelFrame = vi.fn<(id: number) => void>();

beforeEach(() => {
    roInstances = [];
    rafQueue = [];
    clock = 0;
    reads.inHandler = 0;
    cancelFrame.mockReset();
    vi.spyOn(performance, "now").mockImplementation(() => clock);
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        rafQueue.push(cb);
        return rafQueue.length;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => cancelFrame(id));
    vi.spyOn(console, "info").mockImplementation(() => {});
});

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

function setup(geo: Geo = { scrollTop: 0, scrollHeight: 1000, clientHeight: 300 }) {
    const [nodes] = createSignal<DocumentNode[]>([md("a")]);
    const viewState = createAgentViewState(nodes);
    const [docState] = createSignal(emptyDocumentState());
    let jump: ((reason?: string) => void) | undefined;
    const utils = render(() => (
        <AgentDocumentVirtualList
            viewState={viewState}
            documentState={docState}
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
            blockId="abcdef1234567890"
            scrollToBottomRef={(fn) => {
                jump = fn;
            }}
        />
    ));
    const scrollRef = utils.container.querySelector(".agent-document") as HTMLElement;
    const buffer = utils.container.querySelector(".agent-document-streaming-buffer") as HTMLElement;
    const virtualizer = utils.container.querySelector(".agent-document-virtualizer") as HTMLElement;
    const margin = { reads: 0 };
    Object.defineProperty(virtualizer, "offsetTop", {
        configurable: true,
        get: () => {
            if (fromHandler()) margin.reads++;
            return 0;
        },
    });
    const g = makeScrollable(scrollRef, geo);
    triggerResize(buffer); // first observation: pins
    flushRaf();
    reads.inHandler = 0;
    return {
        viewState,
        scrollRef,
        buffer,
        g,
        margin,
        unmount: utils.unmount,
        jump: (r?: string) => jump!(r),
        /** Content grows and the browser runs the ResizeObservers: the pin writes scrollTop, its scroll event is queued. */
        growWithoutFlush(px: number) {
            g.set({ scrollHeight: g.content + px });
            triggerResize(buffer);
        },
        wheelUp() {
            scrollRef.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, bubbles: true }));
        },
        following: () => viewState.stickToBottom(),
    };
}

describe("the scroll handler's batch", () => {
    it("a batch that is only our own pin makes no layout read", () => {
        const s = setup();
        s.growWithoutFlush(200);
        flushRaf();
        expect(reads.inHandler).toBe(0);
        expect(s.margin.reads).toBe(0);
        expect(s.following()).toBe(true);
    });

    it("a batch the user may have caused reads live geometry and re-measures the scroll margin", () => {
        const s = setup();
        s.wheelUp();
        s.g.set({ scrollTop: 100 });
        s.scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();
        expect(reads.inHandler).toBeGreaterThan(0);
        expect(s.margin.reads).toBeGreaterThan(0);
        expect(s.following()).toBe(false);
    });

    it("the pin flag is consumed by its own batch: the next scroll reads live geometry", () => {
        const s = setup();
        s.growWithoutFlush(200);
        flushRaf();
        expect(reads.inHandler).toBe(0);
        s.g.set({ scrollTop: s.g.top - 5 });
        s.scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();
        expect(reads.inHandler).toBeGreaterThan(0);
    });

    it("scroll events within one frame run the handler once, and the next frame can run it again", () => {
        const s = setup();
        const before = rafQueue.length;
        s.scrollRef.dispatchEvent(new Event("scroll"));
        s.scrollRef.dispatchEvent(new Event("scroll"));
        s.scrollRef.dispatchEvent(new Event("scroll"));
        expect(rafQueue.length - before).toBe(1);
        flushRaf();
        s.scrollRef.dispatchEvent(new Event("scroll"));
        expect(rafQueue.length).toBe(1);
    });

    it("unmounting cancels a scroll frame that has not run", () => {
        const s = setup();
        s.scrollRef.dispatchEvent(new Event("scroll"));
        const id = rafQueue.length;
        expect(cancelFrame).not.toHaveBeenCalled();
        s.unmount();
        expect(cancelFrame).toHaveBeenCalledWith(id);
    });

    it("unmounting with no pending scroll frame cancels nothing", () => {
        const s = setup();
        s.unmount();
        expect(cancelFrame).not.toHaveBeenCalled();
    });
});

describe("jump to bottom", () => {
    it("an accepted queued message never pulls a reader who scrolled up back down", () => {
        const s = setup();
        s.wheelUp();
        s.g.set({ scrollTop: 100 });
        s.scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();
        expect(s.following()).toBe(false);
        s.jump("accepted");
        flushRaf();
        expect(s.following()).toBe(false);
        expect(s.g.top).toBe(100);
    });

    it("any other reason follows again and pins", () => {
        const s = setup();
        s.wheelUp();
        s.g.set({ scrollTop: 100 });
        s.scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();
        s.jump("sent");
        flushRaf();
        expect(s.following()).toBe(true);
        expect(s.g.top).toBeGreaterThan(698);
    });
});

describe("unmount", () => {
    it("releases every ResizeObserver the component created", () => {
        setup();
        expect(roInstances.length).toBeGreaterThan(0);
        expect(roInstances.every((r) => !r.disconnected)).toBe(true);
        cleanup();
        expect(roInstances.every((r) => r.disconnected)).toBe(true);
    });
});
