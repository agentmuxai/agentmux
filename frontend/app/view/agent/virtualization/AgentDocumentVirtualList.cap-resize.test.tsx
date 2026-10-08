// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Capped rows' estimates follow the window height (renderers.ts
 * previewCapPx), and a row's estimate is pushed to the layout slice once.
 * Codex P2 on #3934: after a resize, unmeasured rows kept the old window's
 * estimate. The list re-pushes estimates when the cap changes.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { registerPane, snapshot, unregisterPane } from "@/app/store/agent-pane-layout-store";
import type { LayoutView } from "@/app/store/agent-pane-layout/reducer";
import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";
import { STREAMING_BUFFER_SIZE } from "./streaming-buffer";
import type { DocumentNode, DocumentState } from "../types";

class FakeResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
}

const BID = "blk-cap-resize";

beforeEach(() => {
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.stubGlobal("requestAnimationFrame", () => 0);
    vi.stubGlobal("cancelAnimationFrame", () => {});
    vi.stubGlobal("innerHeight", 1400);
    vi.spyOn(console, "info").mockImplementation(() => {});
});
afterEach(() => {
    cleanup();
    unregisterPane(BID);
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

const docState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    heldOpenNodes: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

// A long jekt first (so it lands in the virtualized head, not the streaming
// buffer), then enough rows to fill the buffer.
const jekt: DocumentNode = {
    type: "jekt_message", id: "j0", from: "opaz", to: "agent1", message: "x".repeat(20_000), direction: "incoming",
    tier: "coord", deliveryTier: "wan", trust: "wan-verified", msgId: "m", priority: "normal", timestamp: 0, raw: "",
} as unknown as DocumentNode;
const md = (i: number): DocumentNode => ({ type: "markdown", id: `md${i}`, content: `text ${i}`, timestamp: i });

function mount() {
    const [view, setView] = createSignal<LayoutView | undefined>(undefined);
    registerPane(BID, { layout: setView, zoom: () => {} });
    const [nodes] = createSignal<DocumentNode[]>([jekt, ...Array.from({ length: STREAMING_BUFFER_SIZE + 2 }, (_, i) => md(i))]);
    const viewState = createAgentViewState(nodes);
    const [st] = createSignal(docState());
    render(() => (
        <AgentDocumentVirtualList
            blockId={BID}
            viewState={viewState}
            documentState={st}
            layoutView={view}
            zoomFactor={() => 1}
            tailPolicy="count"
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
        />
    ));
}

const expandedEstimate = (id: string) => snapshot(BID)!.estimates.get(id)?.expanded;

describe("capped estimates follow a window resize", () => {
    it("re-pushes an unmeasured jekt's estimate when the window height changes", async () => {
        mount();
        expect(expandedEstimate("j0")).toBe(290); // 1400 / 6 + 57

        vi.stubGlobal("innerHeight", 2400);
        window.dispatchEvent(new Event("resize"));
        await new Promise((r) => setTimeout(r, 300)); // past the resize debounce
        expect(expandedEstimate("j0")).toBe(457); // 2400 / 6 + 57, not the text estimate's 320 ceiling
    });
});
