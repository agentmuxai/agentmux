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
 * Prepares a container resize the way a real browser lays one out: computes
 * the new geometry and whether a native scrollTop auto-clamp is required,
 * but applies nothing yet. Callers interleave the returned `clamp()` /
 * `notify()` steps themselves (with a `flushRaf()` between them where the
 * test needs one) — this makes the interleaving explicit at the call site
 * instead of hiding it behind an opaque ordering flag. (reagent P2 on the
 * original version of this file: a `clampFirst` boolean that only reordered
 * two side-effecting calls without ever flushing between them produced
 * byte-identical outcomes for both orders, since `notify()`'s own
 * `scrollToTrueBottom()` recomputes and lands on the same clamped value
 * regardless of what `clamp()` already did — so it never actually exercised
 * two different orderings from the SUT's perspective.)
 *
 * `clamp()` is a no-op when the old scrollTop was already a valid position
 * for the new size (true for every shrink, and for a grow that doesn't
 * cross the old scrollTop) — matching real browser behavior, which only
 * adjusts scrollTop when the existing value would otherwise point past the
 * new max.
 */
function prepareResize(
    el: HTMLElement,
    newClientHeight: number,
): { clamp: () => void; notify: () => void } {
    const scrollHeight = el.scrollHeight;
    const before = el.scrollTop;
    setGeometry(el, { clientHeight: newClientHeight });
    const newMax = Math.max(0, scrollHeight - newClientHeight);
    const needsClamp = before > newMax;

    return {
        clamp(): void {
            if (needsClamp) {
                setGeometry(el, { scrollTop: newMax });
                el.dispatchEvent(new Event("scroll"));
            }
        },
        notify(): void {
            triggerResize(el);
        },
    };
}

/** Convenience for tests that don't care about interleaving — apply both
 *  steps back-to-back (clamp, then notify — the real browser order) with no
 *  flush between them. */
function simulateContainerResize(el: HTMLElement, newClientHeight: number): void {
    const resize = prepareResize(el, newClientHeight);
    resize.clamp();
    resize.notify();
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

        simulateContainerResize(scrollRef, 200);
        flushRaf();

        expect(scrollRef.scrollTop).toBe(800); // scrollHeight(1000) - clientHeight(200)
        expect(viewState.stickToBottom()).toBe(true);
    });

    it("stays pinned to true bottom when the pane GROWS", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });

        simulateContainerResize(scrollRef, 500);
        flushRaf();

        expect(scrollRef.scrollTop).toBe(500); // scrollHeight(1000) - clientHeight(500)
        expect(viewState.stickToBottom()).toBe(true);
    });

    it("stays pinned when the pane GROWS and the native clamp's scroll event is fully processed BEFORE the ResizeObserver notifies", () => {
        // The worst-case ordering the (disproven) H1 theory relied on: if the
        // browser's own scrollTop auto-clamp reached handleScrollNow before
        // RO #1 got a chance to also correct, would isNearBottom() read "not
        // near bottom" and disengage stickToBottom? Flushing rAF between
        // clamp() and notify() actually forces that intermediate state
        // through handleScrollNow, rather than letting both scroll events
        // land in the same coalesced flush like the plain "GROWS" test above.
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });

        const resize = prepareResize(scrollRef, 500);
        resize.clamp();
        flushRaf(); // handleScrollNow runs on the clamp alone — RO hasn't fired yet
        expect(scrollRef.scrollTop).toBe(500);
        expect(viewState.stickToBottom()).toBe(true); // gap is already 0 post-clamp — no disengage

        resize.notify(); // RO #1 fires and redundantly re-confirms true bottom
        flushRaf();

        expect(scrollRef.scrollTop).toBe(500);
        expect(viewState.stickToBottom()).toBe(true);
    });

    it("does NOT force-scroll a resize when the user had already scrolled away", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 0 });
        viewState.disengageStickToBottom();

        simulateContainerResize(scrollRef, 200);
        flushRaf();

        expect(scrollRef.scrollTop).toBe(0);
        expect(viewState.stickToBottom()).toBe(false);
    });

    it("survives several rapid shrink ticks within one animation frame (simulated 100Hz drag)", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 1000, clientHeight: 300, scrollTop: 700 });

        // Multiple resize ticks land before a single rAF flush, mirroring a
        // ~100Hz pointer-move drag against a ~60Hz paint cadence.
        simulateContainerResize(scrollRef, 250);
        simulateContainerResize(scrollRef, 200);
        simulateContainerResize(scrollRef, 150);
        flushRaf();

        expect(scrollRef.scrollTop).toBe(850); // scrollHeight(1000) - clientHeight(150)
        expect(viewState.stickToBottom()).toBe(true);
    });
});

// Regression coverage for
// docs/specs/SPEC_AGENT_PANE_FIRST_OVERFLOW_SCROLL_PIN_FIX_2026_08_29.md — a
// pane that starts with no scrollbar (scrollHeight <= clientHeight), then has
// content grow past the viewport for the first time. Every test above this
// point resizes the *container* (clientHeight); these resize *content*
// (scrollHeight) while clientHeight stays fixed — the one geometry
// transition neither this file nor anchor.test.ts previously exercised, and
// exactly the case RO #2 (content-resize observer) exists to catch.
describe("AgentDocumentVirtualList — first-ever overflow", () => {
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

    it("sticks to true bottom when content grows past a previously non-overflowing viewport", () => {
        const { viewState, scrollRef } = setup();
        // No scrollbar yet: content exactly fills the viewport.
        setGeometry(scrollRef, { scrollHeight: 200, clientHeight: 200, scrollTop: 0 });
        expect(viewState.stickToBottom()).toBe(true);

        const virtualContainer = scrollRef.querySelector(".agent-document-virtualizer") as HTMLElement;
        expect(virtualContainer).toBeTruthy();

        // Content resize only — clientHeight is untouched, so RO #1 (viewport
        // resize) never fires; only RO #2 (content resize) can catch this.
        setGeometry(scrollRef, { scrollHeight: 500 });
        triggerResize(virtualContainer);
        flushRaf();

        expect(scrollRef.scrollTop).toBe(300); // scrollHeight(500) - clientHeight(200)
        expect(viewState.stickToBottom()).toBe(true);
    });

    it("does not disengage on a stray scroll event that races the pane's first overflow", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 200, clientHeight: 200, scrollTop: 0 });
        expect(viewState.stickToBottom()).toBe(true);

        // Content has grown past the viewport, but nothing has re-pinned yet
        // — scrollTop is still 0, which now reads as "far from bottom". This
        // mimics a native scroll event landing before RO #2 (or any other
        // re-pin path) gets a chance to run: the exact race §4 of the spec
        // identifies as the failure mode, reproduced directly rather than
        // relying on RO #2 to win the race on its own.
        setGeometry(scrollRef, { scrollHeight: 500 });
        scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();

        // Without the fix: isNearBottom(0, 500, 200) reads false (gap 300px
        // >= the 200px threshold) and this scroll event disengages
        // stickToBottom, stranding the user at the top with no visible
        // indication anything changed until they scroll or type.
        expect(viewState.stickToBottom()).toBe(true);
        expect(scrollRef.scrollTop).toBe(300); // forced to true bottom by the fix
    });

    it("only forces the first transition — a later genuine scroll-away still disengages normally", () => {
        const { viewState, scrollRef } = setup();
        setGeometry(scrollRef, { scrollHeight: 200, clientHeight: 200, scrollTop: 0 });

        // First overflow — latches hasOverflowedOnce and force-pins. The
        // forced scrollToTrueBottom()'s own scrollTo() call reentrantly
        // dispatches a second scroll event that coalesces into a follow-up
        // frame (real `handleScroll` coalescing behavior, not test-only) —
        // flush twice so that settles fully before simulating a distinct,
        // later user action; otherwise the next scroll event below would be
        // misread as still part of this same programmatic batch.
        setGeometry(scrollRef, { scrollHeight: 500 });
        scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();
        flushRaf();
        expect(viewState.stickToBottom()).toBe(true);

        // A later, unambiguous user scroll away from bottom must still
        // disengage normally — the fix only protects the one-time
        // first-overflow transition, not every subsequent scroll event.
        setGeometry(scrollRef, { scrollTop: 0 });
        scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();

        expect(viewState.stickToBottom()).toBe(false);
    });
});
