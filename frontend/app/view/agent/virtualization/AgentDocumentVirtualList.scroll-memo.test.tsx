// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The scroll position comes back after a reload (scroll-memo.ts). Harness as
 * AgentDocumentVirtualList.handoff.test.tsx.
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

const BID = "blk-scroll-memo";
let viewStateRef: ReturnType<typeof createAgentViewState>;
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
    viewStateRef = viewState;
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

// SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §2: a reload puts the pane back where it was.
describe("AgentDocumentVirtualList: the scroll position comes back after a reload", () => {
    it("puts the remembered row at its offset once history is ready, and stops following", () => {
        sessionStorage.setItem(`agentmux:scroll:${BID}`, JSON.stringify({ nodeId: "md5", offsetPx: 7 }));
        vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
            cb();
            return 1;
        });
        const { view, scroller } = setup(STREAMING_BUFFER_SIZE + 40); // md5 is in the virtualized head
        dispatch(BID, { type: "Scrolled", scrollTop: 0, viewportPx: 300 });
        const to = vi.fn();
        (scroller() as unknown as { scrollTo: unknown }).scrollTo = to;
        viewStateRef.markHistoryReady();
        const row = view()!.rows.find((r) => r.nodeId === "md5")!;
        expect(to).toHaveBeenCalledWith({ top: row.start + 7 });
        expect(viewStateRef.stickToBottom()).toBe(false);
        expect(sessionStorage.getItem(`agentmux:scroll:${BID}`)).toBeNull(); // used once
    });

    it("stays at the bottom when the row isn't loaded", () => {
        sessionStorage.setItem(`agentmux:scroll:${BID}`, JSON.stringify({ nodeId: "gone", offsetPx: 7 }));
        vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
            cb();
            return 1;
        });
        const { scroller } = setup(STREAMING_BUFFER_SIZE + 40);
        const to = vi.fn();
        (scroller() as unknown as { scrollTo: unknown }).scrollTo = to;
        viewStateRef.markHistoryReady();
        expect(to).not.toHaveBeenCalled();
        expect(viewStateRef.stickToBottom()).toBe(true);
    });
});
