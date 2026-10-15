// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Rows on screen stay put when rows above them are removed (a live-feed
 * roll-off). Harness as AgentDocumentVirtualList.handoff.test.tsx: jsdom has no
 * layout, row heights come from estimates in the layout store.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { batch, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { dispatch, registerPane, unregisterPane } from "@/app/store/agent-pane-layout-store";
import type { LayoutView } from "@/app/store/agent-pane-layout/reducer";
import { inFlowState, ROW_GAP_PX } from "@/app/store/agent-pane-layout/types";
import { currentExpansion } from "./expansion-source";
import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";
import { STREAMING_BUFFER_SIZE } from "./streaming-buffer";
import type { DocumentNode, DocumentState } from "../types";

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
/** Deliver one RO callback per observer, for every observed target in `els`. */
function resize(els: Element[]): void {
    for (const { callback, targets } of roInstances) {
        const hit = els.filter((el) => targets.has(el));
        if (hit.length) callback(hit.map((target) => ({ target }) as ResizeObserverEntry));
    }
}

const BID = "blk-keep-on-screen";
const rowPx = new Map<string, number>();

beforeEach(() => {
    roInstances = [];
    rowPx.clear();
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal("requestAnimationFrame", () => 0);
    vi.stubGlobal("cancelAnimationFrame", () => {});
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
        const h = rowPx.get((this as HTMLElement).dataset?.nodeId ?? "") ?? 0;
        return { x: 0, y: 0, top: 0, left: 0, bottom: h, right: 400, width: 400, height: h, toJSON: () => ({}) } as DOMRect;
    });
});
afterEach(() => {
    cleanup();
    unregisterPane(BID);
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

const emptyDocumentState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    heldOpenNodes: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

const md = (i: number): DocumentNode => ({ type: "markdown", id: `md${i}`, content: `text ${i}`, timestamp: i });

function setup(count: number) {
    const [view, setView] = createSignal<LayoutView | undefined>(undefined);
    registerPane(BID, { layout: setView, zoom: () => {} });
    const [nodes, setNodes] = createSignal<DocumentNode[]>(Array.from({ length: count }, (_, i) => md(i)));
    const viewState = createAgentViewState(nodes);
    const [docState] = createSignal(emptyDocumentState());
    const utils = render(() => (
        <AgentDocumentVirtualList
            blockId={BID}
            viewState={viewState}
            documentState={docState}
            layoutView={view}
            zoomFactor={() => 1}
            tailPolicy="count"
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
        />
    ));
    const bufferRows = (): HTMLElement[] =>
        [...utils.container.querySelectorAll(".agent-document-streaming-buffer [data-node-id]")] as HTMLElement[];
    const scroller = () => utils.container.querySelector(".agent-document") as HTMLElement;
    return { view, setNodes, scroller };
}

// docs/specs/SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §4.1: rows removed
// above the screen (a live-feed roll-off) must not change which rows are on it.
describe("AgentDocumentVirtualList: rows on screen stay put when rows above are removed", () => {
    it("moves the scroll position with them, in the layout slice and the scroller", () => {
        const HEAD = 40;
        const { view, setNodes, scroller } = setup(STREAMING_BUFFER_SIZE + HEAD); // the first HEAD nodes are the virtualized head
        dispatch(BID, { type: "Scrolled", scrollTop: 900, viewportPx: 300 });
        const onScreen = (v: LayoutView) =>
            v.rows.filter((r) => r.end > snapScrollTop() && r.start < snapScrollTop() + 300).map((r) => r.nodeId);
        let lastTop = 900;
        const snapScrollTop = () => lastTop;
        const before = onScreen(view()!);
        expect(before.length).toBeGreaterThan(0);
        const topRow = view()!.rows.find((r) => r.nodeId === before[0])!;
        const offset = 900 - topRow.start;

        // Roll off the first 10 nodes.
        const removedPx = view()!.rows[10].start - view()!.rows[0].start;
        setNodes((prev) => prev.slice(10));

        const after = view()!;
        lastTop = 900 - removedPx;
        expect(after.rows.find((r) => r.nodeId === before[0])!.start + offset).toBeCloseTo(lastTop);
        expect(onScreen(after)).toEqual(before);
        expect(scroller().scrollTop).toBeCloseTo(lastTop);
        // The window still covers them.
        const ids = after.rows.slice(after.window.startIndex, after.window.endIndex + 1).map((r) => r.nodeId);
        for (const id of before) expect(ids).toContain(id);
    });

    it("leaves the scroll position alone for an append", () => {
        const { view, setNodes } = setup(STREAMING_BUFFER_SIZE + 40);
        dispatch(BID, { type: "Scrolled", scrollTop: 900, viewportPx: 300 });
        const window = view()!.window;
        setNodes((prev) => [...prev, md(1000), md(1001)]);
        expect(view()!.window.startIndex).toBe(window.startIndex);
    });
});
