// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression coverage for the pane-splitter-resize stick-to-bottom bug
 * (docs/specs/PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md). Neither
 * `ResizeObserver` nor `requestAnimationFrame` exist in jsdom, so this file
 * ships small deterministic fakes rather than relying on the browser's real
 * timing — the fakes let each test control exactly when a "resize" is
 * observed and when the resulting `scroll` event is processed, so the two
 * orderings a real drag can produce (native scrollTop clamp before vs. after
 * the ResizeObserver notification) are both exercised explicitly.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";
import type { DocumentNode, DocumentState } from "../types";

afterEach(() => cleanup());

// ── Fake ResizeObserver ─────────────────────────────────────────────────
// Records every `new ResizeObserver(cb)` + the elements each instance
// observes, so a test can simulate "the browser just resized element X" by
// calling triggerResize(el) — which invokes every observer callback that
// currently has X in its observed set, exactly like the real notification
// step would.
type ROCallback = (entries: ResizeObserverEntry[]) => void;
let roInstances: { callback: ROCallback; targets: Set<Element> }[] = [];

class FakeResizeObserver {
    private callback: ROCallback;
    private targets = new Set<Element>();
    constructor(callback: ROCallback) {
        this.callback = callback;
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
    for (const { callback, targets } of roInstances) {
        if (targets.has(el)) {
            callback([{ target: el } as ResizeObserverEntry]);
        }
    }
}

// ── Fake requestAnimationFrame ──────────────────────────────────────────
// Deterministic, manually-flushed queue instead of a real 60Hz timer, so
// tests can assert on state at exact points relative to the scroll-event
// coalescing `AgentDocumentVirtualList` does internally.
let rafQueue: FrameRequestCallback[] = [];
function flushRaf(): void {
    const pending = rafQueue;
    rafQueue = [];
    for (const cb of pending) cb(0);
}

// ── scrollRef geometry helpers ──────────────────────────────────────────
// jsdom has no layout engine, so scrollTop/scrollHeight/clientHeight are
// always 0 and scrollTo() is a no-op. These helpers give the test element
// real, mutable geometry and a scrollTo() that clamps like a real browser.
interface Geometry {
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
}
type PartialGeometry = Partial<Geometry>;

function makeScrollable(el: HTMLElement, geo: Geometry): void {
    let { scrollTop, scrollHeight, clientHeight } = geo;
    Object.defineProperty(el, "scrollTop", {
        configurable: true,
        get: () => scrollTop,
        set: (v: number) => { scrollTop = v; },
    });
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => scrollHeight });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => clientHeight });
    el.scrollTo = ((opts?: ScrollToOptions | number) => {
        const requested: number = typeof opts === "number"
            ? opts
            : (opts?.top ?? scrollTop);
        const max = Math.max(0, scrollHeight - clientHeight);
        scrollTop = Math.max(0, Math.min(requested, max));
        el.dispatchEvent(new Event("scroll"));
    }) as typeof el.scrollTo;
    (el as unknown as { __setGeometry: (g: PartialGeometry) => void }).__setGeometry = (g) => {
        if (g.scrollHeight !== undefined) scrollHeight = g.scrollHeight;
        if (g.clientHeight !== undefined) clientHeight = g.clientHeight;
        if (g.scrollTop !== undefined) scrollTop = g.scrollTop;
    };
}

function setGeometry(el: HTMLElement, g: PartialGeometry): void {
    (el as unknown as { __setGeometry: (g: PartialGeometry) => void }).__setGeometry(g);
}

/**
 * Simulate a real browser laying out a resized scroll container: the
 * scrollTop auto-clamp (if the new size makes the old scrollTop invalid)
 * happens synchronously as part of layout and fires a native `scroll`
 * event; the ResizeObserver notification for the box-size change is a
 * later step. `clampFirst=false` flips that order to cover both.
 */
function simulateContainerResize(
    el: HTMLElement,
    newClientHeight: number,
    opts: { clampFirst: boolean },
): void {
    const scrollHeight = el.scrollHeight;
    const oldMax = Math.max(0, scrollHeight - el.clientHeight);
    const before = el.scrollTop;
    setGeometry(el, { clientHeight: newClientHeight });
    const newMax = Math.max(0, scrollHeight - newClientHeight);
    const needsClamp = before > newMax;

    const doClamp = () => {
        if (needsClamp) {
            setGeometry(el, { scrollTop: newMax });
            el.dispatchEvent(new Event("scroll"));
        }
    };
    const doNotify = () => triggerResize(el);

    if (opts.clampFirst) {
        doClamp();
        doNotify();
    } else {
        doNotify();
        doClamp();
    }
    void oldMax;
}

const emptyDocumentState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    expandedTools: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

function setup() {
    const documentAtom = createSignal<DocumentNode[]>([]);
    const viewState = createAgentViewState(documentAtom);
    const [docState] = createSignal(emptyDocumentState());

    const utils = render(() => (
        <AgentDocumentVirtualList
            viewState={viewState}
            documentState={docState}
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
        />
    ));

    const scrollRef = utils.container.querySelector(".agent-document") as HTMLElement;
    makeScrollable(scrollRef, { scrollTop: 0, scrollHeight: 0, clientHeight: 0 });
    return { viewState, scrollRef };
}

describe("AgentDocumentVirtualList — stick-to-bottom across pane resize", () => {
    beforeEach(() => {
        roInstances = [];
        rafQueue = [];
        vi.stubGlobal("ResizeObserver", FakeResizeObserver);
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            rafQueue.push(cb);
            return rafQueue.length;
        });
        vi.stubGlobal("cancelAnimationFrame", () => {});
    });

    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("stays pinned to true bottom when the pane SHRINKS", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });
        expect(viewState.stickToBottom()).toBe(true);

        simulateContainerResize(scrollRef, 200, { clampFirst: true });
        flushRaf();

        expect(scrollRef.scrollTop).toBe(800); // scrollHeight(1000) - clientHeight(200)
        expect(viewState.stickToBottom()).toBe(true);
    });

    it("stays pinned to true bottom when the pane GROWS, clamp-then-notify order", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });

        simulateContainerResize(scrollRef, 500, { clampFirst: true });
        flushRaf();

        expect(scrollRef.scrollTop).toBe(500); // scrollHeight(1000) - clientHeight(500)
        expect(viewState.stickToBottom()).toBe(true);
    });

    it("stays pinned to true bottom when the pane GROWS, notify-then-clamp order", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });

        simulateContainerResize(scrollRef, 500, { clampFirst: false });
        flushRaf();

        expect(scrollRef.scrollTop).toBe(500);
        expect(viewState.stickToBottom()).toBe(true);
    });

    it("does NOT force-scroll a resize when the user had already scrolled away", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 0 });
        viewState.disengageStickToBottom();

        simulateContainerResize(scrollRef, 200, { clampFirst: true });
        flushRaf();

        expect(scrollRef.scrollTop).toBe(0);
        expect(viewState.stickToBottom()).toBe(false);
    });

    it("survives several rapid shrink ticks within one animation frame (simulated 100Hz drag)", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });

        // Multiple resize ticks land before a single rAF flush, mirroring a
        // ~100Hz pointer-move drag against a ~60Hz paint cadence.
        simulateContainerResize(scrollRef, 250, { clampFirst: true });
        simulateContainerResize(scrollRef, 200, { clampFirst: true });
        simulateContainerResize(scrollRef, 150, { clampFirst: true });
        flushRaf();

        expect(scrollRef.scrollTop).toBe(850); // scrollHeight(1000) - clientHeight(150)
        expect(viewState.stickToBottom()).toBe(true);
    });
});
