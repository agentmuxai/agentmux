// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression coverage for the hidden-pane scroll-off-collapse bug: switching
 * to another browser tab (inactive tab: `display:none`) or minimizing the
 * window makes `getBoundingClientRect()` report an all-zero rect for every
 * element — jsdom's own default `getBoundingClientRect()` already returns
 * all-zero for everything, which is exactly the hidden-pane signature, so
 * these tests exercise it directly without any extra stubbing. Before the
 * fix, `releaseScrolledOffHolds()` treated every held-open tool as
 * "scrolled off" the instant it ran against a zero-size container, even
 * though nothing had actually scrolled.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { AgentDormancyProvider } from "../agent-dormancy";
import { AwayArrivals, CARRY_MAX_MS } from "./held-release";
import { SHOW_SETTLE_MAX_MS, SHOW_SETTLE_MIN_MS, SHOW_SETTLE_SAMPLE_MS } from "./show-settle";
import { createAgentViewState } from "./state";
import type { DocumentNode, DocumentState, ToolNode } from "../types";

afterEach(() => cleanup());

// ── Fake ResizeObserver — AgentDocumentVirtualList registers two of these
// on mount; jsdom has no real one. ─────────────────────────────────────
class FakeResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
}

// ── Fake requestAnimationFrame — scroll handling is rAF-coalesced
// (scrollRafId), so a dispatched "scroll" event needs a manual flush to
// actually reach handleScrollNow. ──────────────────────────────────────
let rafQueue: FrameRequestCallback[] = [];
function flushRaf(): void {
    const pending = rafQueue;
    rafQueue = [];
    for (const cb of pending) cb(0);
}

function makeScrollable(el: HTMLElement, geo: { scrollTop: number; scrollHeight: number; clientHeight: number }): void {
    let { scrollTop, scrollHeight, clientHeight } = geo;
    Object.defineProperty(el, "scrollTop", {
        configurable: true,
        get: () => scrollTop,
        set: (v: number) => { scrollTop = v; },
    });
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => scrollHeight });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => clientHeight });
    el.scrollTo = (() => {}) as typeof el.scrollTo;
}

const toolNode: ToolNode = {
    type: "tool",
    id: "tc-1",
    tool: "Bash",
    params: { command: "ls" },
    status: "success",
    collapsed: true,
    summary: "Bash ls",
};

const documentState = (heldOpenNodes: Set<string>): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    heldOpenNodes,
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

function setup(heldOpenNodes: Set<string>, dormant: () => boolean = () => false, away?: AwayArrivals) {
    const documentAtom = createSignal<DocumentNode[]>([toolNode]);
    const viewState = createAgentViewState(documentAtom[0]);
    const [docState, setDocState] = createSignal(documentState(heldOpenNodes));
    const onReleaseNodeHold = vi.fn();
    const onHoldNodeOpen = vi.fn();

    const utils = render(() => (
        <AgentDormancyProvider dormant={dormant}>
            <AgentDocumentVirtualList
                viewState={viewState}
                documentState={docState}
                onToggleCollapse={() => {}}
                onTogglePin={() => {}}
                onReleaseNodeHold={onReleaseNodeHold}
                onHoldNodeOpen={onHoldNodeOpen}
                awayArrivals={away}
            />
        </AgentDormancyProvider>
    ));

    const scrollRef = utils.container.querySelector(".agent-document") as HTMLElement;
    makeScrollable(scrollRef, { scrollTop: 0, scrollHeight: 500, clientHeight: 200 });
    return { scrollRef, onReleaseNodeHold, onHoldNodeOpen, setDocState };
}

const rect = (top: number, bottom: number): DOMRect =>
    ({ top, bottom, left: 0, right: 800, width: 800, height: bottom - top, x: 0, y: top, toJSON() {} }) as DOMRect;

/** A real-size container (a kept-alive tab under `visibility: hidden` keeps its size) and the row at `row`. */
function place(scrollRef: HTMLElement, row: DOMRect): void {
    scrollRef.getBoundingClientRect = () => rect(0, 200);
    (scrollRef.querySelector('[data-node-id="tc-1"]') as HTMLElement).getBoundingClientRect = () => row;
}

function scroll(scrollRef: HTMLElement): void {
    scrollRef.dispatchEvent(new Event("scroll"));
    flushRaf();
}

describe("AgentDocumentVirtualList — hidden-pane scroll-off collapse guard", () => {
    beforeEach(() => {
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

    it("does NOT release a held-open tool when the container reports a zero-size rect (hidden pane)", () => {
        const { scrollRef, onReleaseNodeHold } = setup(new Set(["tc-1"]));

        // jsdom's default getBoundingClientRect() is already all-zero for
        // every element — exactly the display:none / minimized-window
        // signature — so no extra stubbing is needed to reproduce it.
        scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();

        expect(onReleaseNodeHold).not.toHaveBeenCalled();
    });

    it("still releases a held-open tool once it has genuinely scrolled off the top", () => {
        const { scrollRef, onReleaseNodeHold } = setup(new Set(["tc-1"]));

        // Give the container real, non-zero geometry...
        scrollRef.getBoundingClientRect = () =>
            ({ top: 0, bottom: 200, left: 0, right: 800, width: 800, height: 200, x: 0, y: 0, toJSON() {} }) as DOMRect;
        // ...and the row a rect that has scrolled fully above the top
        // (bottom <= containerTop).
        const row = scrollRef.querySelector('[data-node-id="tc-1"]') as HTMLElement;
        expect(row).not.toBeNull();
        row.getBoundingClientRect = () =>
            ({ top: -50, bottom: -10, left: 0, right: 800, width: 800, height: 40, x: 0, y: -50, toJSON() {} }) as DOMRect;

        scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();

        expect(onReleaseNodeHold).toHaveBeenCalledWith("tc-1");
    });

    // REPORT_AGENT_PANE_HELD_TOOLS_COLLAPSE_ON_RETURN_RCA_2026_10_17.md: leaving
    // the pane and coming back must change nothing.
    it("does NOT release a held-open tool while the pane is hidden at full size (visibility: hidden)", () => {
        const { scrollRef, onReleaseNodeHold } = setup(new Set(["tc-1"]), () => true);
        place(scrollRef, rect(-50, -10)); // pushed off the top while nobody watched
        scroll(scrollRef);
        expect(onReleaseNodeHold).not.toHaveBeenCalled();
    });

    it("keeps a row pushed off while hidden open after the return, until it has been seen again and scrolls off", () => {
        let now = 1_000;
        vi.spyOn(performance, "now").mockImplementation(() => now);
        const [dormant, setDormant] = createSignal(true);
        const { scrollRef, onReleaseNodeHold } = setup(new Set(["tc-1"]), dormant);
        setDormant(false); // shown again
        // Above the top on return (content arrived or was drawn while hidden): kept.
        place(scrollRef, rect(-50, -10));
        now += SHOW_SETTLE_MIN_MS * 10;
        scroll(scrollRef);
        expect(onReleaseNodeHold).not.toHaveBeenCalled();
        // The reader scrolls it back into view: now seen in this visit...
        place(scrollRef, rect(50, 90));
        scroll(scrollRef);
        expect(onReleaseNodeHold).not.toHaveBeenCalled();
        // ...and once it scrolls off the top again, it collapses as designed.
        place(scrollRef, rect(-50, -10));
        scroll(scrollRef);
        expect(onReleaseNodeHold).toHaveBeenCalledWith("tc-1");
    });

    // Rows that finished or arrived while the pane was hidden (nobody watched
    // them, so they weren't held then): the ones on screen once the show has
    // settled are held, as if the reader had watched them land.
    describe("rows that arrived while the pane was hidden", () => {
        beforeEach(() => {
            vi.useFakeTimers();
        });
        afterEach(() => {
            vi.useRealTimers();
            vi.restoreAllMocks();
        });

        function shownWithAway(rowRect: DOMRect | null, away = new AwayArrivals()) {
            let now = 1_000;
            vi.spyOn(performance, "now").mockImplementation(() => now);
            away.add("tc-1");
            const [dormant, setDormant] = createSignal(true);
            const s = setup(new Set(), dormant, away);
            if (rowRect) place(s.scrollRef, rowRect);
            else s.scrollRef.getBoundingClientRect = () => rect(0, 200);
            return {
                ...s,
                away,
                show: () => {
                    setDormant(false);
                    now += SHOW_SETTLE_MIN_MS + 100;
                    vi.advanceTimersByTime(SHOW_SETTLE_MIN_MS + 100);
                },
                hide: () => setDormant(true),
            };
        }

        it("holds the one on screen once the show has settled", () => {
            const t = shownWithAway(rect(50, 90));
            expect(t.onHoldNodeOpen).not.toHaveBeenCalled(); // not while hidden
            t.show();
            expect(t.onHoldNodeOpen).toHaveBeenCalledExactlyOnceWith("tc-1");
            expect(t.away.size).toBe(0);
        });

        it("waits past the minimum while a slow return is still changing the content height, then holds", () => {
            let now = 1_000;
            vi.spyOn(performance, "now").mockImplementation(() => now);
            const away = new AwayArrivals();
            away.add("tc-1");
            const [dormant, setDormant] = createSignal(true);
            const s = setup(new Set(), dormant, away);
            place(s.scrollRef, rect(50, 90));
            let height = 1000;
            Object.defineProperty(s.scrollRef, "scrollHeight", { configurable: true, get: () => height });
            const advance = (ms: number, grow: boolean) => {
                for (let t = 0; t < ms; t += SHOW_SETTLE_SAMPLE_MS) {
                    now += SHOW_SETTLE_SAMPLE_MS;
                    if (grow) height += 100;
                    vi.advanceTimersByTime(SHOW_SETTLE_SAMPLE_MS);
                }
            };
            setDormant(false);
            advance(SHOW_SETTLE_MIN_MS * 3, true); // still drawing long after the old fixed 400 ms
            expect(s.onHoldNodeOpen).not.toHaveBeenCalled();
            advance(SHOW_SETTLE_SAMPLE_MS * 4, false); // it stops
            expect(s.onHoldNodeOpen).toHaveBeenCalledExactlyOnceWith("tc-1");
        });

        it("judges the rows at the cap when the layout never stops", () => {
            let now = 1_000;
            vi.spyOn(performance, "now").mockImplementation(() => now);
            const away = new AwayArrivals();
            away.add("tc-1");
            const [dormant, setDormant] = createSignal(true);
            const s = setup(new Set(), dormant, away);
            place(s.scrollRef, rect(50, 90));
            let height = 1000;
            Object.defineProperty(s.scrollRef, "scrollHeight", { configurable: true, get: () => height });
            setDormant(false);
            for (let t = 0; t < SHOW_SETTLE_MAX_MS - SHOW_SETTLE_SAMPLE_MS; t += SHOW_SETTLE_SAMPLE_MS) {
                now += SHOW_SETTLE_SAMPLE_MS;
                height += 100;
                vi.advanceTimersByTime(SHOW_SETTLE_SAMPLE_MS);
            }
            expect(s.onHoldNodeOpen).not.toHaveBeenCalled();
            for (let t = 0; t < 2 * SHOW_SETTLE_SAMPLE_MS; t += SHOW_SETTLE_SAMPLE_MS) {
                now += SHOW_SETTLE_SAMPLE_MS;
                height += 100;
                vi.advanceTimersByTime(SHOW_SETTLE_SAMPLE_MS);
            }
            expect(s.onHoldNodeOpen).toHaveBeenCalledExactlyOnceWith("tc-1");
        });

        it("leaves one that is already above the viewport to collapse", () => {
            const t = shownWithAway(rect(-50, -10));
            t.show();
            expect(t.onHoldNodeOpen).not.toHaveBeenCalled();
            expect(t.away.size).toBe(0);
        });

        it("ignores an id whose row is gone (removed or replaced)", () => {
            const away = new AwayArrivals();
            away.add("tc-gone");
            const t = shownWithAway(rect(50, 90), away);
            t.show();
            expect(t.onHoldNodeOpen).toHaveBeenCalledExactlyOnceWith("tc-1");
        });

        it("keeps the ids if the pane is hidden again before the show settles, and holds them on the next show", () => {
            const t = shownWithAway(rect(50, 90));
            vi.spyOn(performance, "now").mockImplementation(() => 1_000);
            t.hide();
            vi.advanceTimersByTime(5_000);
            expect(t.onHoldNodeOpen).not.toHaveBeenCalled();
            expect(t.away.size).toBe(1);
        });

        it("holds nothing when the pane reports no size", () => {
            const t = shownWithAway(null);
            t.scrollRef.getBoundingClientRect = () => rect(0, 0);
            t.show();
            expect(t.onHoldNodeOpen).not.toHaveBeenCalled();
        });
    });

    // After a restart or an upgrade the snapshot gives back the rows that were
    // open (`restoredHeldNodes`): carried, so they aren't closed because the
    // restored transcript is drawn above the viewport.
    describe("rows a snapshot restored as held", () => {
        afterEach(() => {
            vi.restoreAllMocks();
        });

        it("stay open until seen on screen in this visit, then collapse when they scroll off", () => {
            let now = 1_000;
            vi.spyOn(performance, "now").mockImplementation(() => now);
            const { scrollRef, onReleaseNodeHold, setDocState } = setup(new Set(["tc-1"]));
            setDocState((prev) => ({ ...prev, restoredHeldNodes: new Set(["tc-1"]) }));
            place(scrollRef, rect(-50, -10)); // the restored transcript put it above the viewport
            now += SHOW_SETTLE_MAX_MS + 1; // the restore has settled
            scroll(scrollRef);
            expect(onReleaseNodeHold).not.toHaveBeenCalled();
            place(scrollRef, rect(50, 90)); // the reader scrolls it into view: seen
            scroll(scrollRef);
            place(scrollRef, rect(-50, -10));
            scroll(scrollRef);
            expect(onReleaseNodeHold).toHaveBeenCalledWith("tc-1");
        });

        it("are released after CARRY_MAX_MS if never seen, like any carried row", () => {
            let now = 1_000;
            vi.spyOn(performance, "now").mockImplementation(() => now);
            const { scrollRef, onReleaseNodeHold, setDocState } = setup(new Set(["tc-1"]));
            setDocState((prev) => ({ ...prev, restoredHeldNodes: new Set(["tc-1"]) }));
            place(scrollRef, rect(-50, -10));
            now += CARRY_MAX_MS - 1;
            scroll(scrollRef);
            expect(onReleaseNodeHold).not.toHaveBeenCalled();
            now += 2;
            scroll(scrollRef);
            expect(onReleaseNodeHold).toHaveBeenCalledWith("tc-1");
        });

        it("without the restore marker the same row is released at once (the baseline)", () => {
            const { scrollRef, onReleaseNodeHold } = setup(new Set(["tc-1"]));
            place(scrollRef, rect(-50, -10));
            scroll(scrollRef);
            expect(onReleaseNodeHold).toHaveBeenCalledWith("tc-1");
        });
    });

    it("doesn't count a row as seen while the return is still settling", () => {
        let now = 1_000;
        vi.spyOn(performance, "now").mockImplementation(() => now);
        const [dormant, setDormant] = createSignal(true);
        const { scrollRef, onReleaseNodeHold } = setup(new Set(["tc-1"]), dormant);
        setDormant(false);
        // On screen during the catch-up, then pushed off by it.
        place(scrollRef, rect(50, 90));
        scroll(scrollRef);
        place(scrollRef, rect(-50, -10));
        now += SHOW_SETTLE_MIN_MS * 10;
        scroll(scrollRef);
        expect(onReleaseNodeHold).not.toHaveBeenCalled();
    });
});
