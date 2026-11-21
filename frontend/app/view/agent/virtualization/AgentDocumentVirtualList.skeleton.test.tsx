// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Rows just outside the mounted window are drawn as empty boxes (skeleton-band.ts), so a scroll that
 * outruns the frames meets a row's shape, not a hole. A box is not a row: it carries no node id (nothing
 * measures it or finds it as a row).
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { dispatch, registerPane, unregisterPane } from "@/app/store/agent-pane-layout-store";
import type { LayoutView } from "@/app/store/agent-pane-layout/reducer";
import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";
import type { DocumentNode, DocumentState } from "../../../agent-model/types";

const BID = "blk-skeleton";

beforeEach(() => {
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    vi.stubGlobal("requestAnimationFrame", () => 0);
    vi.stubGlobal("cancelAnimationFrame", () => {});
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
const turn = (k: number): DocumentNode[] => [
    { type: "user_message", id: `u${k}`, message: `u${k}`, timestamp: 0 },
    { type: "markdown", id: `a${k}`, content: `text ${k}`, timestamp: 0 },
];

describe("skeleton rows", () => {
    it("fill the band past the window and carry no node id", () => {
        const [view, setView] = createSignal<LayoutView | undefined>(undefined);
        registerPane(BID, { layout: setView, zoom: () => {} });
        const [nodes] = createSignal<DocumentNode[]>(Array.from({ length: 80 }, (_, k) => turn(k)).flat());
        const [state] = createSignal(docState());
        const { container } = render(() => (
            <AgentDocumentVirtualList
                blockId={BID}
                viewState={createAgentViewState(nodes)}
                documentState={state}
                layoutView={view}
                zoomFactor={() => 1}
                onToggleCollapse={() => {}}
                onTogglePin={() => {}}
            />
        ));
        dispatch(BID, { type: "Scrolled", scrollTop: 3000, viewportPx: 400 });

        const skeletons = [...container.querySelectorAll(".agent-row-skeleton")] as HTMLElement[];
        const mounted = [...container.querySelectorAll(".agent-document-virtualizer [data-node-id]")];
        expect(skeletons.length).toBeGreaterThan(0);
        expect(mounted.length).toBeGreaterThan(0);
        for (const s of skeletons) {
            expect(s.dataset.nodeId).toBeUndefined();
            expect(s.getAttribute("aria-hidden")).toBe("true");
            expect(parseFloat(s.style.height)).toBeGreaterThan(0);
        }
        // A band, not the whole document (skeleton-band.test.ts checks that no window row is in it).
        expect(skeletons.length).toBeLessThan(160);
    });
});
