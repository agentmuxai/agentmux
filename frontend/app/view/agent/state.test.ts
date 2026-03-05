// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach } from "vitest";
import { createStore } from "jotai";
import {
    createAgentAtoms,
    createFilteredDocumentAtom,
    createDocumentStatsAtom,
    createToggleNodeCollapsed,
    createAppendDocumentNode,
    createClearDocument,
    type AgentAtoms,
} from "./state";
import type { DocumentNode, MarkdownNode, ToolNode } from "./types";

let store: ReturnType<typeof createStore>;
let atoms: AgentAtoms;

beforeEach(() => {
    store = createStore();
    atoms = createAgentAtoms("test-block-1");
});

describe("createAgentAtoms", () => {
    test("creates atoms with correct default values", () => {
        expect(store.get(atoms.documentAtom)).toEqual([]);
        expect(store.get(atoms.rawOutputAtom)).toBe("");
        expect(store.get(atoms.sessionIdAtom)).toBe("");
        expect(store.get(atoms.authAtom)).toEqual({ status: "disconnected" });
        expect(store.get(atoms.userInfoAtom)).toBeNull();
        expect(store.get(atoms.providerConfigAtom)).toBeNull();
    });

    test("rawOutputAtom can be updated", () => {
        store.set(atoms.rawOutputAtom, "hello world");
        expect(store.get(atoms.rawOutputAtom)).toBe("hello world");
    });

    test("rawOutputAtom can accumulate output", () => {
        store.set(atoms.rawOutputAtom, "line 1\n");
        store.set(atoms.rawOutputAtom, store.get(atoms.rawOutputAtom) + "line 2\n");
        expect(store.get(atoms.rawOutputAtom)).toBe("line 1\nline 2\n");
    });

    test("authAtom defaults to disconnected", () => {
        const auth = store.get(atoms.authAtom);
        expect(auth.status).toBe("disconnected");
    });

    test("authAtom can be set to connected", () => {
        store.set(atoms.authAtom, { status: "connected" });
        expect(store.get(atoms.authAtom).status).toBe("connected");
    });

    test("documentStateAtom has correct default filter", () => {
        const state = store.get(atoms.documentStateAtom);
        expect(state.filter.showThinking).toBe(false);
        expect(state.filter.showSuccessfulTools).toBe(true);
        expect(state.filter.showFailedTools).toBe(true);
        expect(state.filter.showIncoming).toBe(true);
        expect(state.filter.showOutgoing).toBe(true);
    });

    test("separate instances have independent state", () => {
        const atoms2 = createAgentAtoms("test-block-2");
        store.set(atoms.rawOutputAtom, "instance 1");
        store.set(atoms2.rawOutputAtom, "instance 2");
        expect(store.get(atoms.rawOutputAtom)).toBe("instance 1");
        expect(store.get(atoms2.rawOutputAtom)).toBe("instance 2");
    });
});

// Helper to create test nodes
function makeMarkdownNode(id: string, content: string, thinking = false): MarkdownNode {
    return {
        type: "markdown",
        id,
        content,
        collapsed: false,
        metadata: thinking ? { thinking: true } : undefined,
    } as MarkdownNode;
}

function makeToolNode(id: string, tool: string, status: "running" | "success" | "failed"): ToolNode {
    return {
        type: "tool",
        id,
        tool,
        params: {},
        status,
        collapsed: false,
    } as ToolNode;
}

describe("createFilteredDocumentAtom", () => {
    test("returns all nodes when no filters active", () => {
        const filtered = createFilteredDocumentAtom(atoms.documentAtom, atoms.documentStateAtom);

        const nodes: DocumentNode[] = [
            makeMarkdownNode("1", "Hello"),
            makeToolNode("2", "bash", "success"),
        ];
        store.set(atoms.documentAtom, nodes);

        expect(store.get(filtered)).toHaveLength(2);
    });

    test("filters thinking nodes when showThinking is false", () => {
        const filtered = createFilteredDocumentAtom(atoms.documentAtom, atoms.documentStateAtom);

        const nodes: DocumentNode[] = [
            makeMarkdownNode("1", "Hello"),
            makeMarkdownNode("2", "Thinking...", true),
        ];
        store.set(atoms.documentAtom, nodes);

        // showThinking defaults to false
        const result = store.get(filtered);
        expect(result).toHaveLength(1);
        expect(result[0].id).toBe("1");
    });

    test("shows thinking nodes when filter enabled", () => {
        const filtered = createFilteredDocumentAtom(atoms.documentAtom, atoms.documentStateAtom);

        const nodes: DocumentNode[] = [
            makeMarkdownNode("1", "Hello"),
            makeMarkdownNode("2", "Thinking...", true),
        ];
        store.set(atoms.documentAtom, nodes);

        const state = store.get(atoms.documentStateAtom);
        store.set(atoms.documentStateAtom, {
            ...state,
            filter: { ...state.filter, showThinking: true },
        });

        expect(store.get(filtered)).toHaveLength(2);
    });

    test("filters successful tools when showSuccessfulTools is false", () => {
        const filtered = createFilteredDocumentAtom(atoms.documentAtom, atoms.documentStateAtom);

        const nodes: DocumentNode[] = [
            makeToolNode("1", "bash", "success"),
            makeToolNode("2", "bash", "failed"),
        ];
        store.set(atoms.documentAtom, nodes);

        const state = store.get(atoms.documentStateAtom);
        store.set(atoms.documentStateAtom, {
            ...state,
            filter: { ...state.filter, showSuccessfulTools: false },
        });

        const result = store.get(filtered);
        expect(result).toHaveLength(1);
        expect(result[0].id).toBe("2");
    });
});

describe("createDocumentStatsAtom", () => {
    test("returns zero counts for empty document", () => {
        const stats = createDocumentStatsAtom(atoms.documentAtom);
        const result = store.get(stats);
        expect(result.totalNodes).toBe(0);
        expect(result.markdownNodes).toBe(0);
        expect(result.toolNodes).toBe(0);
    });

    test("counts nodes by type", () => {
        const stats = createDocumentStatsAtom(atoms.documentAtom);

        store.set(atoms.documentAtom, [
            makeMarkdownNode("1", "Hello"),
            makeMarkdownNode("2", "World"),
            makeToolNode("3", "bash", "success"),
            makeToolNode("4", "read", "failed"),
            makeToolNode("5", "write", "running"),
        ]);

        const result = store.get(stats);
        expect(result.totalNodes).toBe(5);
        expect(result.markdownNodes).toBe(2);
        expect(result.toolNodes).toBe(3);
        expect(result.successfulTools).toBe(1);
        expect(result.failedTools).toBe(1);
        expect(result.runningTools).toBe(1);
    });
});

describe("createToggleNodeCollapsed", () => {
    test("toggles node collapsed state", () => {
        const toggle = createToggleNodeCollapsed(atoms.documentStateAtom);

        // Initially empty
        expect(store.get(atoms.documentStateAtom).collapsedNodes.size).toBe(0);

        // Collapse node
        store.set(toggle, "node-1");
        expect(store.get(atoms.documentStateAtom).collapsedNodes.has("node-1")).toBe(true);

        // Uncollapse node
        store.set(toggle, "node-1");
        expect(store.get(atoms.documentStateAtom).collapsedNodes.has("node-1")).toBe(false);
    });
});

describe("createClearDocument", () => {
    test("clears document and resets state", () => {
        const clear = createClearDocument(atoms.documentAtom, atoms.documentStateAtom);

        // Add some data
        store.set(atoms.documentAtom, [makeMarkdownNode("1", "Hello")]);
        expect(store.get(atoms.documentAtom)).toHaveLength(1);

        // Clear
        store.set(clear);
        expect(store.get(atoms.documentAtom)).toEqual([]);
        expect(store.get(atoms.documentStateAtom).collapsedNodes.size).toBe(0);
    });
});
