// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The streaming buffer holds only the turn in flight — Phase 3 of
 * docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.2.
 *
 * Covers where the frontier lands (the last user message, earlier nodes still
 * in progress, the ceilings — unit-tested in streaming-buffer.test.ts) as the
 * LIST applies it: straight to the target when nothing is mounted yet, only
 * across rows wholly above the viewport afterwards, past the viewport rule
 * once the tail is over twice a ceiling, never backwards, and never with a
 * node in both regions (invariant 1).
 *
 * Geometry is what the list itself uses — observed buffer-row heights (fake
 * ResizeObserver + stubbed rect, as in the handoff test), the layout store's
 * scrollTop and head size — so nothing here needs real layout.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { dispatch, registerPane, snapshot, unregisterPane } from "@/app/store/agent-pane-layout-store";
import type { LayoutView } from "@/app/store/agent-pane-layout/reducer";
import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";
import { STREAMING_BUFFER_SIZE, TURN_TAIL_MAX_NODES } from "./streaming-buffer";
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
function resize(els: Element[]): void {
    for (const { callback, targets } of roInstances) {
        const hit = els.filter((el) => targets.has(el));
        if (hit.length) callback(hit.map((target) => ({ target }) as ResizeObserverEntry));
    }
}

const BID = "blk-turn-tail";
const ROW = 50; // every row's observed height

beforeEach(() => {
    roInstances = [];
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal("requestAnimationFrame", () => 0);
    vi.stubGlobal("cancelAnimationFrame", () => {});
    vi.spyOn(console, "info").mockImplementation(() => {});
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
        const h = (this as HTMLElement).dataset?.nodeId ? ROW : 0;
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

const user = (id: string): DocumentNode => ({ type: "user_message", id, message: id, timestamp: 0 });
const md = (id: string): DocumentNode => ({ type: "markdown", id, content: `text ${id}`, timestamp: 0 });
const tool = (id: string, status: "running" | "success" = "success"): DocumentNode =>
    ({ type: "tool", id, tool: "Bash", params: {}, status, collapsed: true, summary: id }) as DocumentNode;
const turn = (k: number): DocumentNode[] => [user(`u${k}`), md(`a${k}`), tool(`t${k}`)];

function setup(initial: DocumentNode[], opts: { blockId?: string; tailPolicy?: "turn" | "count" } = { blockId: BID }) {
    const [view, setView] = createSignal<LayoutView | undefined>(undefined);
    if (opts.blockId) registerPane(opts.blockId, { layout: setView, zoom: () => {} });
    const [nodes, setNodes] = createSignal<DocumentNode[]>(initial);
    const [docState] = createSignal(emptyDocumentState());
    const utils = render(() => (
        <AgentDocumentVirtualList
            blockId={opts.blockId}
            viewState={createAgentViewState(nodes)}
            documentState={docState}
            layoutView={view}
            zoomFactor={() => 1}
            tailPolicy={opts.tailPolicy}
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
        />
    ));
    const tailIds = (): string[] =>
        [...utils.container.querySelectorAll(".agent-document-streaming-buffer > [data-node-id]")].map((e) => (e as HTMLElement).dataset.nodeId!);
    const headIds = (): string[] => [...(snapshot(BID)?.orderedIds ?? [])];
    /** The browser lays out the buffer: every buffer row reports ROW px. */
    const measureTail = () =>
        resize([...utils.container.querySelectorAll(".agent-document-streaming-buffer > [data-node-id]")]);
    const scrollTo = (scrollTop: number) => dispatch(BID, { type: "Scrolled", scrollTop, viewportPx: 400 });
    /** Invariant 1: a node id is never in both regions. */
    const assertSingleResidency = () => {
        const head = new Set(headIds());
        for (const id of tailIds()) expect(head.has(id), `${id} in both head and tail`).toBe(false);
    };
    return { setNodes, tailIds, headIds, measureTail, scrollTo, assertSingleResidency };
}

describe("turn-scoped streaming buffer", () => {
    it("mounting a conversation keeps only its last turn in the buffer", () => {
        const s = setup([...turn(1), ...turn(2), ...turn(3)]);
        expect(s.tailIds()).toEqual(["u3", "a3", "t3"]);
        expect(s.headIds()).toEqual(["u1", "a1", "t1", "u2", "a2", "t2"]);
        s.assertSingleResidency();
    });

    it("the same without a layout store (no geometry to protect): straight to the turn", () => {
        const s = setup([...turn(1), ...turn(2)], {});
        expect(s.tailIds()).toEqual(["u2", "a2", "t2"]);
    });

    it("a new turn moves the previous one into the head once its rows are above the viewport", () => {
        const s = setup(turn(1));
        expect(s.tailIds()).toEqual(["u1", "a1", "t1"]);
        s.measureTail();
        s.scrollTo(1000); // all three rows (0..162) are above the viewport
        s.setNodes((n) => [...n, user("u2")]);
        expect(s.tailIds()).toEqual(["u2"]);
        expect(s.headIds()).toEqual(["u1", "a1", "t1"]);
        s.assertSingleResidency();
    });

    it("a previous turn still on screen stays mounted; its rows follow once they scroll off the top", () => {
        const s = setup(turn(1));
        s.measureTail();
        s.scrollTo(60); // u1 (0..50) is above; a1 (54..104) is on screen
        s.setNodes((n) => [...n, user("u2")]);
        expect(s.tailIds()).toEqual(["a1", "t1", "u2"]);
        expect(s.headIds()).toEqual(["u1"]);
        s.assertSingleResidency();

        s.measureTail();
        s.scrollTo(1000);
        s.setNodes((n) => [...n, md("a2")]); // any later update retries the move
        expect(s.tailIds()).toEqual(["u2", "a2"]);
        expect(s.headIds()).toEqual(["u1", "a1", "t1"]);
        s.assertSingleResidency();
    });

    it("an earlier tool still running keeps itself and everything after it mounted", () => {
        const s = setup([user("u1"), tool("t1", "running"), md("a1")]);
        s.measureTail();
        s.scrollTo(1000);
        s.setNodes((n) => [...n, user("u2")]);
        expect(s.tailIds()).toEqual(["t1", "a1", "u2"]);
        expect(s.headIds()).toEqual(["u1"]);
    });

    it("past twice the node ceiling the tail is trimmed even with nothing measured and the rows on screen", () => {
        const long = Array.from({ length: 2 * TURN_TAIL_MAX_NODES }, (_, i) => md(`m${i}`));
        const s = setup([user("u1"), ...long.slice(0, 5)]);
        s.scrollTo(0); // everything on screen, nothing measured
        s.setNodes([user("u1"), ...long]); // 81 nodes: over 2× the ceiling
        expect(s.tailIds().length).toBe(TURN_TAIL_MAX_NODES);
        s.assertSingleResidency();
    });

    it("the frontier never moves backwards, even if a node already in the head reads as in progress", () => {
        // Not a real transition (a finished tool does not restart), but the
        // guard matters: moving back would pull a head node into the buffer,
        // the migration direction the June replaceChild crashes lived in.
        const s = setup([...turn(1), ...turn(2)]);
        expect(s.headIds()).toEqual(["u1", "a1", "t1"]);
        s.setNodes((n) => [n[0], n[1], tool("t1", "running"), ...n.slice(3), md("a2b")]);
        expect(s.headIds()).toEqual(["u1", "a1", "t1"]);
        expect(s.tailIds()).toEqual(["u2", "a2", "t2", "a2b"]);
        s.assertSingleResidency();
    });

    it("the count policy (kill switch) keeps the last STREAMING_BUFFER_SIZE nodes, as before Phase 3", () => {
        const nodes = Array.from({ length: 20 }, (_, k) => turn(k)).flat(); // 60 nodes
        const s = setup(nodes, { blockId: BID, tailPolicy: "count" });
        expect(s.tailIds().length).toBe(STREAMING_BUFFER_SIZE);
        expect(s.headIds().length).toBe(60 - STREAMING_BUFFER_SIZE);
    });
});
