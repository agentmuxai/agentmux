// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Height handoff when a node migrates from the streaming buffer into the
 * virtualized head — Phase 3 of
 * docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.2
 * (invariant 3, "no jump").
 *
 * A node in the buffer is laid out by the browser; once it moves into the
 * head it is placed by the layout store's prefix sums. If the store only has
 * an estimate for it on that first frame, everything below it moves by the
 * estimate's error until the head's measure RO corrects it a frame later.
 * The buffer rows' heights are observed (ResizeObserver, no forced layout)
 * so the migrating node enters the store with its real height in the same
 * update that adds it.
 *
 * jsdom has no layout: row heights come from a stubbed getBoundingClientRect
 * keyed by data-node-id, delivered through a fake ResizeObserver — the same
 * approach as AgentDocumentVirtualList.pin.test.tsx.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { dispatch, registerPane, unregisterPane } from "@/app/store/agent-pane-layout-store";
import type { LayoutView } from "@/app/store/agent-pane-layout/reducer";
import { ROW_GAP_PX } from "@/app/store/agent-pane-layout/types";
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

const BID = "blk-handoff";
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
    expandedTools: new Set(),
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
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
        />
    ));
    const bufferRows = (): HTMLElement[] =>
        [...utils.container.querySelectorAll(".agent-document-streaming-buffer [data-node-id]")] as HTMLElement[];
    return { view, setNodes, bufferRows };
}

describe("height handoff from the streaming buffer to the virtualized head", () => {
    it("a migrating node enters the layout store with its measured height, not an estimate", () => {
        const s = setup(STREAMING_BUFFER_SIZE);
        const rows = s.bufferRows();
        expect(rows.length).toBe(STREAMING_BUFFER_SIZE);
        for (const el of rows) rowPx.set(el.dataset.nodeId!, 77);
        resize(rows); // the browser lays the buffer out and reports sizes

        s.setNodes((n) => [...n, md(STREAMING_BUFFER_SIZE)]); // md0 migrates into the head

        const v = s.view();
        expect(v?.rows.map((r) => r.nodeId)).toEqual(["md0"]);
        expect(v?.rows[0].height).toBe(77);
        // The head now occupies exactly the space md0 had in the buffer: its
        // height plus the one gap after it. Nothing below moves.
        expect(v?.totalSize).toBe(77 + ROW_GAP_PX);
    });

    it("a node that was never measured in the buffer still enters with an estimate (and is corrected by the head's measure RO)", () => {
        const s = setup(STREAMING_BUFFER_SIZE);
        s.setNodes((n) => [...n, md(STREAMING_BUFFER_SIZE)]);
        const v = s.view();
        expect(v?.rows.map((r) => r.nodeId)).toEqual(["md0"]);
        expect(v?.rows[0].height).toBeGreaterThan(0);
        expect(v?.rows[0].height).not.toBe(77);
    });

    it("a height handed off once is not re-applied over a later head measurement", () => {
        const s = setup(STREAMING_BUFFER_SIZE);
        const rows = s.bufferRows();
        for (const el of rows) rowPx.set(el.dataset.nodeId!, 77);
        resize(rows);
        s.setNodes((n) => [...n, md(STREAMING_BUFFER_SIZE)]);
        expect(s.view()?.rows[0].height).toBe(77);

        // md0 is now a head row; give the store a viewport so it is mounted,
        // then the browser measures it at a new height.
        dispatch(BID, { type: "Scrolled", scrollTop: 0, viewportPx: 600 });
        const headRow = document.querySelector(".agent-document-virtualizer [data-node-id='md0']") as HTMLElement;
        expect(headRow).not.toBeNull();
        rowPx.set("md0", 90);
        resize([headRow]);
        expect(s.view()?.rows[0].height).toBe(90);

        // More migrations must not resurrect md0's stale buffer height.
        s.setNodes((n) => [...n, md(STREAMING_BUFFER_SIZE + 1)]);
        expect(s.view()?.rows.find((r) => r.nodeId === "md0")?.height).toBe(90);
    });
});
