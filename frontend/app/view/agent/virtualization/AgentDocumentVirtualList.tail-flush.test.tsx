// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A flush that changes only the live tail costs the same however long the transcript is.
 *
 * Every flush hands the list a new node array. The head of it (the rows above the streaming buffer) is
 * the same nodes in the same order, and the work that follows the head (pushing every row's order,
 * estimates and expansion into the layout store, rebuilding the node-by-id map, putting the rows on
 * screen back where they were) must not run for it. It is counted here as the layout store's
 * `NodesChanged` pushes, one per run of that work, with no layout needed.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const nodesChanged = vi.hoisted(() => ({ count: 0 }));
vi.mock("@/app/store/agent-pane-layout-store", async (importOriginal) => {
    const real = await importOriginal<typeof import("@/app/store/agent-pane-layout-store")>();
    return {
        ...real,
        dispatchIfRegistered: (...args: Parameters<typeof real.dispatchIfRegistered>) => {
            if (args[1].type === "NodesChanged") nodesChanged.count++;
            return real.dispatchIfRegistered(...args);
        },
    };
});

import { registerPane, unregisterPane } from "@/app/store/agent-pane-layout-store";
import type { LayoutView } from "@/app/store/agent-pane-layout/reducer";
import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";
import type { DocumentNode, DocumentState } from "../../../agent-model/types";

const BID = "blk-tail-flush";

beforeEach(() => {
    nodesChanged.count = 0;
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
const user = (id: string): DocumentNode => ({ type: "user_message", id, message: id, timestamp: 0 });
const md = (id: string, content = `text ${id}`): DocumentNode => ({ type: "markdown", id, content, timestamp: 0 });
const tool = (id: string): DocumentNode =>
    ({ type: "tool", id, tool: "Bash", params: {}, status: "success", collapsed: true, summary: id }) as DocumentNode;
const turn = (k: number): DocumentNode[] => [user(`u${k}`), md(`a${k}`), tool(`t${k}`)];

function setup(turns: number) {
    const [view, setView] = createSignal<LayoutView | undefined>(undefined);
    registerPane(BID, { layout: setView, zoom: () => {} });
    const initial = Array.from({ length: turns }, (_, k) => turn(k)).flat();
    const [nodes, setNodes] = createSignal<DocumentNode[]>(initial);
    const [state] = createSignal(docState());
    render(() => (
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
    return { setNodes };
}

describe("a flush that changes only the live tail", () => {
    it.each([20, 200])("does not redo the head's work, with %i turns above it", (turns) => {
        const s = setup(turns);
        const last = turns - 1;
        const base = nodesChanged.count;
        for (let i = 0; i < 25; i++) {
            // One streamed delta of the last answer: a new array, the same head, one new tail node.
            s.setNodes((n) => [...n.slice(0, -1), md(`a${last}`, `text a${last} ${"x".repeat(i)}`)]);
        }
        expect(nodesChanged.count - base).toBe(0);
    });

    it("still redoes it when a head row changes", () => {
        const s = setup(20);
        const base = nodesChanged.count;
        s.setNodes((n) => n.map((x, i) => (i === 4 ? md("a1", "changed in place") : x)));
        expect(nodesChanged.count - base).toBe(1);
    });

    it("still redoes it when a row is added to or removed from the head", () => {
        const s = setup(20);
        const base = nodesChanged.count;
        s.setNodes((n) => n.filter((x) => x.id !== "t3"));
        expect(nodesChanged.count - base).toBe(1);
    });
});
