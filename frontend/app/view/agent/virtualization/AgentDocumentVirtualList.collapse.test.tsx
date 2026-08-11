// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression coverage for the hidden-pane scroll-off-collapse bug: switching
 * to another browser tab (inactive tab: `display:none`) or minimizing the
 * window makes `getBoundingClientRect()` report an all-zero rect for every
 * element — jsdom's own default `getBoundingClientRect()` already returns
 * all-zero for everything, which is exactly the hidden-pane signature, so
 * these tests exercise it directly without any extra stubbing. Before the
 * fix, `collapseScrolledOffTools()` treated every held-open tool as
 * "scrolled off" the instant it ran against a zero-size container, even
 * though nothing had actually scrolled.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
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

const documentState = (expandedTools: Set<string>): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    expandedTools,
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

function setup(expandedTools: Set<string>) {
    const documentAtom = createSignal<DocumentNode[]>([toolNode]);
    const viewState = createAgentViewState(documentAtom);
    const [docState] = createSignal(documentState(expandedTools));
    const onReleaseToolOpen = vi.fn();

    const utils = render(() => (
        <AgentDocumentVirtualList
            viewState={viewState}
            documentState={docState}
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
            onReleaseToolOpen={onReleaseToolOpen}
        />
    ));

    const scrollRef = utils.container.querySelector(".agent-document") as HTMLElement;
    makeScrollable(scrollRef, { scrollTop: 0, scrollHeight: 500, clientHeight: 200 });
    return { scrollRef, onReleaseToolOpen };
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
        const { scrollRef, onReleaseToolOpen } = setup(new Set(["tc-1"]));

        // jsdom's default getBoundingClientRect() is already all-zero for
        // every element — exactly the display:none / minimized-window
        // signature — so no extra stubbing is needed to reproduce it.
        scrollRef.dispatchEvent(new Event("scroll"));
        flushRaf();

        expect(onReleaseToolOpen).not.toHaveBeenCalled();
    });

    it("still releases a held-open tool once it has genuinely scrolled off the top", () => {
        const { scrollRef, onReleaseToolOpen } = setup(new Set(["tc-1"]));

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

        expect(onReleaseToolOpen).toHaveBeenCalledWith("tc-1");
    });
});
